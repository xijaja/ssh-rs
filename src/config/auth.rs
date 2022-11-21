use crate::algorithm::hash::HashCtx;
use crate::algorithm::hash::HashType;
use crate::model::Data;
use crate::{algorithm::hash, constant::algorithms};
use crate::{SshError, SshResult};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::PublicKeyParts;
use std::fmt::Debug;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const KEY_FILE_MAGIC_START: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";

#[derive(Clone, Default)]
pub struct KeyPair {
    pub(super) private_key: String,
    pub(super) key_type: KeyType,
}

impl KeyPair {
    pub fn from_str(key_str: &str) -> SshResult<Self> {
        // first validate the key
        let key_str = key_str.trim().to_owned();

        let (key_type, private_key) = if rsa::RsaPrivateKey::from_pkcs1_pem(&key_str).is_ok() {
            (KeyType::PemRsa, key_str)
        } else if key_str.starts_with(KEY_FILE_MAGIC_START) {
            match ssh_key::PrivateKey::from_openssh(&key_str) {
                Ok(prk) => match prk.algorithm() {
                    ssh_key::Algorithm::Rsa { hash: _hash } => (KeyType::SshRsa, key_str),
                    ssh_key::Algorithm::Ed25519 => (KeyType::SshEd25519, key_str),
                    x => {
                        return Err(SshError::from(format!(
                            "Currently don't support the key file type {}",
                            x
                        )))
                    }
                },
                Err(e) => return Err(SshError::from(e.to_string())),
            }
        } else {
            return Err(SshError::from("Unable to detect the pulic key type"));
        };

        // then store it
        let pair = KeyPair {
            private_key,
            key_type,
        };
        Ok(pair)
    }

    pub fn get_blob(&self, alg: &str) -> Vec<u8> {
        match self.key_type {
            KeyType::PemRsa => {
                // already valid key string, just unwrap it.
                let rprk = rsa::RsaPrivateKey::from_pkcs1_pem(&self.private_key).unwrap();
                let rpuk = rprk.to_public_key();
                let es = rpuk.e().to_bytes_be();
                let ns = rpuk.n().to_bytes_be();
                let mut blob = Data::new();
                blob.put_str(alg);
                blob.put_mpint(&es);
                blob.put_mpint(&ns);
                blob.to_vec()
            }
            KeyType::SshRsa => {
                let prk = ssh_key::PrivateKey::from_openssh(&self.private_key).unwrap();
                let rpuk = prk.key_data().rsa().unwrap();
                let es = rpuk.public.e.as_bytes();
                let ns = rpuk.public.n.as_bytes();
                let mut blob = Data::new();
                blob.put_str(alg);
                blob.put_mpint(es);
                blob.put_mpint(ns);
                blob.to_vec()
            }
            KeyType::SshEd25519 => {
                unreachable!()
            }
        }
    }

    fn sign(&self, sd: &[u8], alg: &str) -> Vec<u8> {
        let (scheme, digest) = match alg {
            algorithms::pubkey::RSA_SHA2_512 => (
                rsa::PaddingScheme::new_pkcs1v15_sign::<sha2::Sha512>(),
                ring::digest::digest(&ring::digest::SHA512, sd),
            ),
            algorithms::pubkey::RSA_SHA2_256 => (
                rsa::PaddingScheme::new_pkcs1v15_sign::<sha2::Sha256>(),
                ring::digest::digest(&ring::digest::SHA256, sd),
            ),
            #[cfg(feature = "dangerous-rsa-sha1")]
            algorithms::pubkey::SSH_RSA => (
                rsa::PaddingScheme::new_pkcs1v15_sign::<sha1::Sha1>(),
                ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, sd),
            ),
            _ => todo!(),
        };
        let msg = digest.as_ref();

        match self.key_type {
            KeyType::PemRsa => {
                let rprk = rsa::RsaPrivateKey::from_pkcs1_pem(self.private_key.as_str()).unwrap();

                rprk.sign(scheme, msg).unwrap()
            }
            KeyType::SshRsa => {
                let prk = ssh_key::PrivateKey::from_openssh(&self.private_key).unwrap();
                let rsa = prk.key_data().rsa().unwrap();
                let rprk = rsa::RsaPrivateKey::try_from(rsa).unwrap();

                rprk.sign(scheme, msg).unwrap()
            }
            KeyType::SshEd25519 => {
                unreachable!()
            }
        }
    }

    pub(crate) fn signature(
        &self,
        buf: &[u8],
        hash_ctx: HashCtx,
        hash_type: HashType,
        alg: &str,
    ) -> Vec<u8> {
        let session_id = hash::digest(hash_ctx.as_bytes().as_slice(), hash_type);
        let mut sd = Data::new();
        sd.put_u8s(session_id.as_slice());
        sd.extend_from_slice(buf);
        let sign = self.sign(&sd, alg);
        let mut ss = Data::new();
        ss.put_str(alg);
        ss.put_u8s(&sign);
        ss.to_vec()
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub(super) enum KeyType {
    PemRsa,
    SshRsa,
    SshEd25519,
}

impl Default for KeyType {
    fn default() -> Self {
        KeyType::PemRsa
    }
}

#[derive(Clone, Default)]
pub(crate) struct AuthInfo {
    pub username: String,
    pub password: String,
    pub key_pair: Option<KeyPair>,
}

impl Debug for AuthInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "username: {}", self.username)?;
        Ok(())
    }
}

impl AuthInfo {
    pub fn username<U>(&mut self, u: U) -> SshResult<()>
    where
        U: ToString,
    {
        self.username = u.to_string();
        Ok(())
    }

    pub fn password<P>(&mut self, p: P) -> SshResult<()>
    where
        P: ToString,
    {
        self.password = p.to_string();
        Ok(())
    }

    pub fn private_key<K>(&mut self, k: K) -> SshResult<()>
    where
        K: ToString,
    {
        self.key_pair = Some((KeyPair::from_str(&k.to_string()))?);
        Ok(())
    }

    pub fn private_key_path<P>(&mut self, p: P) -> SshResult<()>
    where
        P: AsRef<Path>,
    {
        let mut file = match File::open(p) {
            Ok(file) => file,
            Err(e) => return Err(SshError::from(e.to_string())),
        };
        let mut prks = String::new();
        file.read_to_string(&mut prks)?;

        self.key_pair = Some((KeyPair::from_str(&prks))?);
        Ok(())
    }
}
