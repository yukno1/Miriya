//! PIE（Play In Editor）会话状态机。
//!
//! 会话由编辑器核心驱动，UI 宿主只发送状态请求；核心不启动进程、窗口或渲染。

use thiserror::Error;

/// PIE 会话状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PieState {
    #[default]
    Stopped,
    Starting,
    Running,
    Paused,
    Stopping,
}

impl PieState {
    /// 是否为合法状态迁移。
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Stopped, Self::Starting)
                | (Self::Starting, Self::Running)
                | (Self::Starting, Self::Stopping)
                | (Self::Running, Self::Paused)
                | (Self::Running, Self::Stopping)
                | (Self::Paused, Self::Running)
                | (Self::Paused, Self::Stopping)
                | (Self::Stopping, Self::Stopped)
        )
    }
}

/// 非法 PIE 状态迁移。
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PieError {
    #[error("invalid PIE transition: {from:?} -> {to:?}")]
    InvalidTransition { from: PieState, to: PieState },
}

/// PIE 会话。
#[derive(Debug, Default)]
pub struct PieSession {
    state: PieState,
}

impl PieSession {
    /// 创建已停止的会话。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前状态。
    pub fn state(&self) -> PieState {
        self.state
    }

    /// 是否处于运行相关状态。
    pub fn is_active(&self) -> bool {
        !matches!(self.state, PieState::Stopped)
    }

    /// 开始启动流程。
    pub fn begin(&mut self) -> Result<(), PieError> {
        self.transition(PieState::Starting)
    }

    /// 启动完成，进入运行。
    pub fn started(&mut self) -> Result<(), PieError> {
        self.transition(PieState::Running)
    }

    /// 暂停。
    pub fn pause(&mut self) -> Result<(), PieError> {
        self.transition(PieState::Paused)
    }

    /// 从暂停恢复。
    pub fn resume(&mut self) -> Result<(), PieError> {
        self.transition(PieState::Running)
    }

    /// 请求停止。
    pub fn stop(&mut self) -> Result<(), PieError> {
        self.transition(PieState::Stopping)
    }

    /// 停止完成，回到已停止。
    pub fn finish(&mut self) -> Result<(), PieError> {
        self.transition(PieState::Stopped)
    }

    fn transition(&mut self, to: PieState) -> Result<(), PieError> {
        if !self.state.can_transition_to(to) {
            return Err(PieError::InvalidTransition {
                from: self.state,
                to,
            });
        }
        self.state = to;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_pie_lifecycle() {
        let mut session = PieSession::new();
        assert_eq!(session.state(), PieState::Stopped);
        session.begin().unwrap();
        session.started().unwrap();
        session.pause().unwrap();
        session.resume().unwrap();
        session.stop().unwrap();
        session.finish().unwrap();
        assert_eq!(session.state(), PieState::Stopped);
        assert!(!session.is_active());
    }

    #[test]
    fn invalid_transition_is_rejected() {
        let mut session = PieSession::new();
        assert!(session.pause().is_err());
        assert_eq!(session.state(), PieState::Stopped);
    }
}
