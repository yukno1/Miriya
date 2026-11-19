//! 空物理后端：供 CI、服务器与未接入外部后端时使用。
//!
//! 不实现任何求解逻辑；仅维护句柄有效性与刚体变换，让引擎主循环与契约测试可运行。

use std::collections::HashMap;

use mir_math::Vec3;

use crate::backend::{
    BodyTransform, ColliderDesc, ColliderShape, ContactPair, DebugShape, FixedStep, OverlapHit,
    PhysicsBackend, PhysicsError, Ray, RayHit, RigidBodyDesc, SweepHit,
};
use crate::handle::{BodyId, ColliderId, Generation};

/// 不做任何物理求解的空后端。
#[derive(Debug, Default)]
pub struct NullPhysicsBackend {
    world_created: bool,
    next_body_slot: u32,
    next_collider_slot: u32,
    bodies: HashMap<BodyId, BodyTransform>,
    velocities: HashMap<BodyId, (Vec3, Vec3)>,
    colliders: HashMap<ColliderId, BodyId>,
}

impl NullPhysicsBackend {
    /// 创建空后端。
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_world(&self) -> Result<(), PhysicsError> {
        if self.world_created {
            Ok(())
        } else {
            Err(PhysicsError::WorldNotCreated)
        }
    }
}

impl PhysicsBackend for NullPhysicsBackend {
    fn create_world(&mut self, _gravity: Vec3) -> Result<(), PhysicsError> {
        if self.world_created {
            return Err(PhysicsError::WorldAlreadyCreated);
        }
        self.world_created = true;
        Ok(())
    }

    fn destroy_world(&mut self) -> Result<(), PhysicsError> {
        self.ensure_world()?;
        self.world_created = false;
        self.bodies.clear();
        self.velocities.clear();
        self.colliders.clear();
        Ok(())
    }

    fn add_body(&mut self, desc: RigidBodyDesc) -> Result<BodyId, PhysicsError> {
        self.ensure_world()?;
        let id = BodyId::new(self.next_body_slot, Generation::ZERO);
        self.next_body_slot = self.next_body_slot.wrapping_add(1);
        self.bodies.insert(
            id,
            BodyTransform {
                position: desc.position,
                rotation: desc.rotation,
            },
        );
        self.velocities
            .insert(id, (desc.linear_velocity, desc.angular_velocity));
        Ok(id)
    }

    fn remove_body(&mut self, body: BodyId) -> Result<(), PhysicsError> {
        self.ensure_world()?;
        if self.bodies.remove(&body).is_none() {
            return Err(PhysicsError::InvalidBody(body));
        }
        self.velocities.remove(&body);
        self.colliders.retain(|_, owner| *owner != body);
        Ok(())
    }

    fn set_body_transform(
        &mut self,
        body: BodyId,
        transform: BodyTransform,
    ) -> Result<(), PhysicsError> {
        self.ensure_world()?;
        let slot = self
            .bodies
            .get_mut(&body)
            .ok_or(PhysicsError::InvalidBody(body))?;
        *slot = transform;
        Ok(())
    }

    fn body_transform(&self, body: BodyId) -> Result<BodyTransform, PhysicsError> {
        self.ensure_world()?;
        self.bodies
            .get(&body)
            .copied()
            .ok_or(PhysicsError::InvalidBody(body))
    }

    fn set_body_velocity(
        &mut self,
        body: BodyId,
        linear: Vec3,
        angular: Vec3,
    ) -> Result<(), PhysicsError> {
        self.ensure_world()?;
        let slot = self
            .velocities
            .get_mut(&body)
            .ok_or(PhysicsError::InvalidBody(body))?;
        *slot = (linear, angular);
        Ok(())
    }

    fn add_collider(
        &mut self,
        body: BodyId,
        _desc: ColliderDesc,
    ) -> Result<ColliderId, PhysicsError> {
        self.ensure_world()?;
        if !self.bodies.contains_key(&body) {
            return Err(PhysicsError::InvalidBody(body));
        }
        let id = ColliderId::new(self.next_collider_slot, Generation::ZERO);
        self.next_collider_slot = self.next_collider_slot.wrapping_add(1);
        self.colliders.insert(id, body);
        Ok(id)
    }

    fn remove_collider(&mut self, collider: ColliderId) -> Result<(), PhysicsError> {
        self.ensure_world()?;
        self.colliders
            .remove(&collider)
            .map(|_| ())
            .ok_or(PhysicsError::InvalidCollider(collider))
    }

    fn step(&mut self, step: FixedStep) -> Result<(), PhysicsError> {
        self.ensure_world()?;
        if !step.is_valid() {
            return Err(PhysicsError::InvalidFixedStep(step.as_secs()));
        }
        Ok(())
    }

    fn drain_contact_events(&mut self) -> Vec<ContactPair> {
        Vec::new()
    }

    fn raycast(&self, _ray: Ray, _max_distance: f32) -> Result<Option<RayHit>, PhysicsError> {
        self.ensure_world()?;
        Ok(None)
    }

    fn sweep(
        &self,
        _shape: ColliderShape,
        _transform: BodyTransform,
        _direction: Vec3,
        _max_distance: f32,
    ) -> Result<Option<SweepHit>, PhysicsError> {
        self.ensure_world()?;
        Ok(None)
    }

    fn overlap(
        &self,
        _shape: ColliderShape,
        _transform: BodyTransform,
    ) -> Result<Vec<OverlapHit>, PhysicsError> {
        self.ensure_world()?;
        Ok(Vec::new())
    }

    fn debug_draw(&self) -> Vec<DebugShape> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::assert_backend_contract;

    #[test]
    fn null_backend_passes_contract() {
        assert_backend_contract(&mut NullPhysicsBackend::new());
    }

    #[test]
    fn queries_require_world() {
        let backend = NullPhysicsBackend::new();
        assert!(matches!(
            backend.raycast(
                Ray {
                    origin: Vec3::ZERO,
                    direction: Vec3::Z
                },
                1.0
            ),
            Err(PhysicsError::WorldNotCreated)
        ));
    }
}
