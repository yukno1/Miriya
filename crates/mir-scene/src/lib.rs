//! 场景子系统：游戏对象的组织与管理。

use mir_ecs::World;
use mir_math::Vec3;

/// 场景：一个 World 加上场景级元数据。
pub struct Scene {
    pub name: String,
    pub world: World,
    ambient: Vec3,
}

impl Scene {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            world: World::new(),
            ambient: Vec3::ONE,
        }
    }

    /// 环境光颜色。
    pub fn ambient(&self) -> Vec3 {
        self.ambient
    }

    pub fn set_ambient(&mut self, color: Vec3) {
        self.ambient = color;
    }
}

/// 内置的变换组件，几乎所有对象都需要。
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: Vec3,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

impl mir_ecs::Component for Transform {}
