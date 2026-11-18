//! Miriya Engine —— 门面 crate。
//!
//! 引擎本体按子系统拆分为多个独立 crate（位于 `crates/engine/crates/`）：
//!
//! | crate       | 职责                     |
//! |-------------|--------------------------|
//! | `mir-core`  | 主循环、生命周期、配置   |
//! | `mir-math`  | 数学库                   |
//! | `mir-ecs`   | 实体组件系统             |
//! | `mir-render`| 渲染后端抽象             |
//! | `mir-input` | 输入状态管理             |
//! | `mir-asset` | 资源加载与管理           |
//! | `mir-scene` | 场景与游戏对象           |
//! | `mir-window`| 窗口抽象                 |
//! | `mir-log`   | 日志系统                 |
//!
//! 本 crate 将它们统一重新导出，应用层只需依赖 `miriya-engine`；
//! 需要精细控制依赖时也可以直接引用子 crate。

pub use mir_asset as asset;
pub use mir_core as core;
pub use mir_ecs as ecs;
pub use mir_input as input;
pub use mir_log as log;
pub use mir_math as math;
pub use mir_render as render;
pub use mir_scene as scene;
pub use mir_window as window;

pub use mir_core::{Engine, EngineConfig};
