//! Miriya 编辑器核心：与 UI 框架无关的状态机。
//!
//! 只管理关卡文档、选择、命令/撤销、Inspector 描述、视口数学与 PIE 会话；
//! **不依赖** winit、Vulkan、egui 或 GPUI（README §3.5 / §5.7 / §9.15）。
//! UI 宿主把事件翻译成 [`Command`] 交给 [`EditorCore::apply`]，并读取核心状态呈现；
//! 切换 UI 后端无需修改本 crate。
//!
//! ```
//! use mir_editor_core::{EditorCore, LevelDocument, ObjectId, SceneObject, Command};
//! use mir_math::Vec3;
//!
//! let mut core = EditorCore::new();
//! core.open_document(LevelDocument::new("level"));
//! core.apply(Command::AddObject {
//!     object: SceneObject {
//!         id: ObjectId::new(1),
//!         name: "Chair".into(),
//!         position: Vec3::ZERO,
//!     },
//! }).unwrap();
//! core.undo().unwrap();
//! assert!(core.document().unwrap().objects().is_empty());
//! ```

#![forbid(unsafe_code)]

pub mod command;
pub mod document;
pub mod inspector;
pub mod pie;
pub mod viewport;

pub use command::{Command, CommandError, CommandStack};
pub use document::{LevelDocument, ObjectId, SceneObject, Selection};
pub use inspector::{InspectorField, InspectorFieldKind, InspectorModel};
pub use pie::{PieError, PieSession, PieState};
pub use viewport::{GizmoAxis, GizmoMode, PickRequest, PickResult, ViewportCamera};

/// 编辑器核心状态。可在无窗口、无 GPU 的 headless 环境完整运行。
#[derive(Debug, Default)]
pub struct EditorCore {
    document: Option<LevelDocument>,
    selection: Selection,
    commands: CommandStack,
    pie: PieSession,
    viewport: ViewportCamera,
    inspector: InspectorModel,
}

impl EditorCore {
    /// 创建空核心（无打开文档）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 打开关卡；同时重置选择、命令栈与 PIE 会话。
    pub fn open_document(&mut self, document: LevelDocument) {
        self.document = Some(document);
        self.selection.clear();
        self.commands.clear();
        self.pie = PieSession::new();
        self.inspector.clear();
    }

    /// 关闭当前关卡。
    pub fn close_document(&mut self) {
        self.document = None;
        self.selection.clear();
        self.commands.clear();
        self.pie = PieSession::new();
        self.inspector.clear();
    }

    /// 当前关卡文档。
    pub fn document(&self) -> Option<&LevelDocument> {
        self.document.as_ref()
    }

    /// 当前选择集。
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// 命令栈（供 UI 查询撤销/重做可用性）。
    pub fn commands(&self) -> &CommandStack {
        &self.commands
    }

    /// PIE 会话。
    pub fn pie(&self) -> &PieSession {
        &self.pie
    }

    /// 可变 PIE 会话，供核心驱动状态迁移。
    pub fn pie_mut(&mut self) -> &mut PieSession {
        &mut self.pie
    }

    /// 视口相机。
    pub fn viewport(&self) -> &ViewportCamera {
        &self.viewport
    }

    /// 可变视口相机。
    pub fn viewport_mut(&mut self) -> &mut ViewportCamera {
        &mut self.viewport
    }

    /// Inspector 字段描述。
    pub fn inspector(&self) -> &InspectorModel {
        &self.inspector
    }

    /// 可变 Inspector 字段描述。
    pub fn inspector_mut(&mut self) -> &mut InspectorModel {
        &mut self.inspector
    }

    /// 执行一条命令并压入撤销栈。
    pub fn apply(&mut self, command: Command) -> Result<(), CommandError> {
        self.execute(&command)?;
        self.commands.push(command);
        Ok(())
    }

    /// 撤销上一条命令。
    pub fn undo(&mut self) -> Result<(), CommandError> {
        let inverse = self.commands.pop_undo().ok_or(CommandError::EmptyUndo)?;
        self.execute(&inverse)
    }

    /// 重做上一条撤销的命令。
    pub fn redo(&mut self) -> Result<(), CommandError> {
        let command = self.commands.pop_redo().ok_or(CommandError::EmptyRedo)?;
        self.execute(&command)
    }

    /// 执行命令本身（不入栈），供撤销/重做复用。
    fn execute(&mut self, command: &Command) -> Result<(), CommandError> {
        let document = self.document.as_mut().ok_or(CommandError::NoDocument)?;

        match command {
            Command::Select { new, .. } => {
                for id in new {
                    if !document.contains(*id) {
                        return Err(CommandError::ObjectNotFound(*id));
                    }
                }
                self.selection = Selection::from_ids(new.iter().copied());
            }
            Command::AddObject { object } => {
                if !document.add_object(object.clone()) {
                    return Err(CommandError::DuplicateObject(object.id));
                }
            }
            Command::RemoveObject { object } => {
                if document.remove_object(object.id).is_none() {
                    return Err(CommandError::ObjectNotFound(object.id));
                }
                self.selection.remove(object.id);
            }
            Command::RenameObject { id, new, .. } => {
                let target = document
                    .object_mut(*id)
                    .ok_or(CommandError::ObjectNotFound(*id))?;
                target.name = new.clone();
            }
            Command::MoveObject { id, new, .. } => {
                let target = document
                    .object_mut(*id)
                    .ok_or(CommandError::ObjectNotFound(*id))?;
                target.position = *new;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mir_math::Vec3;

    fn object(id: u64, name: &str) -> SceneObject {
        SceneObject {
            id: ObjectId::new(id),
            name: name.to_string(),
            position: Vec3::ZERO,
        }
    }

    #[test]
    fn commands_require_open_document() {
        let mut core = EditorCore::new();
        assert!(matches!(
            core.apply(Command::AddObject {
                object: object(1, "Chair")
            }),
            Err(CommandError::NoDocument)
        ));
    }

    #[test]
    fn add_rename_move_undo_redo() {
        let mut core = EditorCore::new();
        core.open_document(LevelDocument::new("level"));

        core.apply(Command::AddObject {
            object: object(1, "Chair"),
        })
        .unwrap();
        core.apply(Command::RenameObject {
            id: ObjectId::new(1),
            old: "Chair".to_string(),
            new: "Table".to_string(),
        })
        .unwrap();
        core.apply(Command::MoveObject {
            id: ObjectId::new(1),
            old: Vec3::ZERO,
            new: Vec3::new(10.0, 0.0, 0.0),
        })
        .unwrap();

        let doc = core.document().unwrap();
        assert_eq!(doc.object(ObjectId::new(1)).unwrap().name, "Table");
        assert_eq!(
            doc.object(ObjectId::new(1)).unwrap().position,
            Vec3::new(10.0, 0.0, 0.0)
        );

        core.undo().unwrap();
        assert_eq!(
            core.document()
                .unwrap()
                .object(ObjectId::new(1))
                .unwrap()
                .position,
            Vec3::ZERO
        );
        core.undo().unwrap();
        assert_eq!(
            core.document()
                .unwrap()
                .object(ObjectId::new(1))
                .unwrap()
                .name,
            "Chair"
        );
        core.undo().unwrap();
        assert!(core.document().unwrap().objects().is_empty());
        assert!(!core.commands().can_undo());

        core.redo().unwrap();
        core.redo().unwrap();
        core.redo().unwrap();
        assert_eq!(
            core.document()
                .unwrap()
                .object(ObjectId::new(1))
                .unwrap()
                .name,
            "Table"
        );
        assert!(!core.commands().can_redo());
    }

    #[test]
    fn selection_undo_restores_previous() {
        let mut core = EditorCore::new();
        core.open_document(LevelDocument::new("level"));
        core.apply(Command::AddObject {
            object: object(1, "A"),
        })
        .unwrap();
        core.apply(Command::AddObject {
            object: object(2, "B"),
        })
        .unwrap();

        core.apply(Command::Select {
            old: Vec::new(),
            new: vec![ObjectId::new(1)],
        })
        .unwrap();
        core.apply(Command::Select {
            old: vec![ObjectId::new(1)],
            new: vec![ObjectId::new(2)],
        })
        .unwrap();
        assert!(core.selection().contains(ObjectId::new(2)));

        core.undo().unwrap();
        assert!(core.selection().contains(ObjectId::new(1)));
        assert!(!core.selection().contains(ObjectId::new(2)));
    }

    #[test]
    fn remove_object_invalidates_handle_and_selection() {
        let mut core = EditorCore::new();
        core.open_document(LevelDocument::new("level"));
        let target = object(1, "Chair");
        core.apply(Command::AddObject {
            object: target.clone(),
        })
        .unwrap();
        core.apply(Command::Select {
            old: Vec::new(),
            new: vec![target.id],
        })
        .unwrap();
        core.apply(Command::RemoveObject {
            object: target.clone(),
        })
        .unwrap();
        assert!(core.selection().is_empty());
        assert!(core.document().unwrap().object(target.id).is_none());

        core.undo().unwrap();
        assert_eq!(core.document().unwrap().objects().len(), 1);
    }

    #[test]
    fn headless_core_needs_no_ui_backend() {
        let mut core = EditorCore::new();
        core.open_document(LevelDocument::new("level"));
        assert_eq!(core.pie().state(), PieState::Stopped);
        assert!(core.viewport().fov_y_degrees > 0.0);
        assert_eq!(core.pie_mut().state(), PieState::Stopped);
        assert!(core.inspector().is_empty());
    }

    #[test]
    fn duplicate_object_is_rejected() {
        let mut core = EditorCore::new();
        core.open_document(LevelDocument::new("level"));
        core.apply(Command::AddObject {
            object: object(1, "A"),
        })
        .unwrap();
        assert!(matches!(
            core.apply(Command::AddObject {
                object: object(1, "B")
            }),
            Err(CommandError::DuplicateObject(id)) if id == ObjectId::new(1)
        ));
    }
}
