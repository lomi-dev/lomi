//! Windows emulator.exe waits for a separate QEMU child. Own both before the
//! launcher can execute, and retain ownership until every job member exits.
use lomi_control_core::process_tree::{Membership, Tree};
use std::{
    io,
    process::{Command, ExitStatus},
};

pub(super) struct Child {
    tree: Tree,
    pub stdout: Option<tokio::fs::File>,
    pub stderr: Option<tokio::fs::File>,
}

impl Child {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        use std::os::windows::io::OwnedHandle;
        let mut tree = Tree::spawn(command)?;
        let stdout =
            tree.child.stdout.take().map(|pipe| {
                tokio::fs::File::from_std(std::fs::File::from(OwnedHandle::from(pipe)))
            });
        let stderr =
            tree.child.stderr.take().map(|pipe| {
                tokio::fs::File::from_std(std::fs::File::from(OwnedHandle::from(pipe)))
            });
        Ok(Self {
            tree,
            stdout,
            stderr,
        })
    }

    pub fn id(&self) -> Option<u32> {
        Some(self.tree.child.id())
    }

    pub fn membership(&self) -> Membership {
        self.tree.membership()
    }

    pub fn start_kill(&mut self) -> io::Result<()> {
        self.tree.kill()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.tree.try_wait()
    }
}
