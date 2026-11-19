//! 类型化编辑命令与撤销/重做栈。
//!
//! 不使用 UE 式 `UObject` 命令继承体系：命令是可枚举的 Rust 值，
//! [`Command::invert`] 生成逆命令，核心负责执行与入栈。

use mir_math::Vec3;
use thiserror::Error;

use crate::document::{ObjectId, SceneObject};

/// 编辑命令。每项都携带撤销所需的旧值。
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Select {
        old: Vec<ObjectId>,
        new: Vec<ObjectId>,
    },
    AddObject {
        object: SceneObject,
    },
    RemoveObject {
        object: SceneObject,
    },
    RenameObject {
        id: ObjectId,
        old: String,
        new: String,
    },
    MoveObject {
        id: ObjectId,
        old: Vec3,
        new: Vec3,
    },
}

impl Command {
    /// 生成逆命令。
    pub fn invert(&self) -> Self {
        match self {
            Command::Select { old, new } => Command::Select {
                old: new.clone(),
                new: old.clone(),
            },
            Command::AddObject { object } => Command::RemoveObject {
                object: object.clone(),
            },
            Command::RemoveObject { object } => Command::AddObject {
                object: object.clone(),
            },
            Command::RenameObject { id, old, new } => Command::RenameObject {
                id: *id,
                old: new.clone(),
                new: old.clone(),
            },
            Command::MoveObject { id, old, new } => Command::MoveObject {
                id: *id,
                old: *new,
                new: *old,
            },
        }
    }
}

/// 命令执行错误。
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CommandError {
    #[error("no level document is open")]
    NoDocument,
    #[error("object not found: {0}")]
    ObjectNotFound(ObjectId),
    #[error("duplicate object id: {0}")]
    DuplicateObject(ObjectId),
    #[error("undo stack is empty")]
    EmptyUndo,
    #[error("redo stack is empty")]
    EmptyRedo,
}

/// 撤销/重做栈。只存储命令，不接触 UI 或渲染。
#[derive(Debug, Default)]
pub struct CommandStack {
    undo: Vec<Command>,
    redo: Vec<Command>,
}

impl CommandStack {
    /// 创建空栈。
    pub fn new() -> Self {
        Self::default()
    }

    /// 压入新命令并清空重做栈。
    pub fn push(&mut self, command: Command) {
        self.undo.push(command);
        self.redo.clear();
    }

    /// 弹出撤销项，返回需要执行的逆命令。
    pub fn pop_undo(&mut self) -> Option<Command> {
        let command = self.undo.pop()?;
        self.redo.push(command.clone());
        Some(command.invert())
    }

    /// 弹出重做项，返回需要重新执行的命令。
    pub fn pop_redo(&mut self) -> Option<Command> {
        let command = self.redo.pop()?;
        self.undo.push(command.clone());
        Some(command)
    }

    /// 是否可撤销。
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// 是否可重做。
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// 可撤销命令数。
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// 可重做命令数。
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// 清空两个栈。
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invert_round_trips() {
        let command = Command::MoveObject {
            id: ObjectId::new(1),
            old: Vec3::ZERO,
            new: Vec3::X,
        };
        assert_eq!(command.invert().invert(), command);
    }

    #[test]
    fn push_clears_redo() {
        let mut stack = CommandStack::new();
        let command = Command::Select {
            old: Vec::new(),
            new: Vec::new(),
        };
        stack.push(command.clone());
        assert!(stack.pop_undo().is_some());
        assert!(stack.can_redo());
        stack.push(command);
        assert!(!stack.can_redo());
    }
}
