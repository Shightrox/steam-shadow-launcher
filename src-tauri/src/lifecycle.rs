//! Drain in-flight workspace operations before changing its location.
use std::{
    path::PathBuf,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};
static WORKSPACE: RwLock<()> = RwLock::new(());
pub struct Workspace {
    path: PathBuf,
    _guard: RwLockReadGuard<'static, ()>,
}
impl Workspace {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            _guard: read(),
        }
    }
}
impl std::ops::Deref for Workspace {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.path
    }
}
pub fn read() -> RwLockReadGuard<'static, ()> {
    WORKSPACE.read().unwrap()
}
pub fn write() -> RwLockWriteGuard<'static, ()> {
    WORKSPACE.write().unwrap()
}
