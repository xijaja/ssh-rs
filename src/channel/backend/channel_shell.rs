use super::channel::ChannelBroker;
use crate::constant::{ssh_msg_code, ssh_str};
use crate::error::SshResult;
use crate::model::Data;
use std::ops::{Deref, DerefMut};

pub struct ShellBrocker(ChannelBroker);

impl ShellBrocker {
    pub(crate) fn open(channel: ChannelBroker) -> SshResult<Self> {
        // shell 形式需要一个伪终端
        let mut channel_shell = ShellBrocker(channel);
        channel_shell.request_pty()?;
        channel_shell.get_shell()?;
        Ok(channel_shell)
    }

    fn request_pty(&mut self) -> SshResult<()> {
        let mut data = Data::new();
        data.put_u8(ssh_msg_code::SSH_MSG_CHANNEL_REQUEST)
            .put_u32(self.server_channel_no)
            .put_str(ssh_str::PTY_REQ)
            .put_u8(true as u8)
            .put_str(ssh_str::XTERM_VAR)
            .put_u32(80)
            .put_u32(24)
            .put_u32(640)
            .put_u32(480);
        let model = [
            128, // TTY_OP_ISPEED
            0, 1, 0xc2, 0,   // 115200
            129, // TTY_OP_OSPEED
            0, 1, 0xc2, 0,    // 115200 again
            0_u8, // TTY_OP_END
        ];
        data.put_u8s(&model);
        self.send(data)
    }

    fn get_shell(&mut self) -> SshResult<()> {
        let mut data = Data::new();
        data.put_u8(ssh_msg_code::SSH_MSG_CHANNEL_REQUEST)
            .put_u32(self.server_channel_no)
            .put_str(ssh_str::SHELL)
            .put_u8(true as u8);
        self.send(data)
    }

    /// this method will try to read as much data as we can from the server,
    /// but it will block until at least one packet is received
    ///
    pub fn read(&mut self) -> SshResult<Vec<u8>> {
        let mut out = self.recv()?;
        while let Ok(Some(mut data)) = self.try_recv() {
            out.append(&mut data)
        }
        Ok(out)
    }

    /// this method send `buf` to the remote pty
    ///
    pub fn write(&mut self, buf: &[u8]) -> SshResult<()> {
        self.send_data(buf.to_vec().into())?;
        Ok(())
    }

    pub fn close(self) -> SshResult<()> {
        self.0.close()
    }
}

impl Deref for ShellBrocker {
    type Target = ChannelBroker;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ShellBrocker {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
