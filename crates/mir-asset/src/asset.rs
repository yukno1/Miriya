//! 资产类型契约、纯数据枚举与句柄。

use std::marker::PhantomData;

use mir_image::{ImageData, Ktx2Data};
use mir_render::{MaterialData, MeshData};

use crate::AssetId;

/// 音频 PCM 数据（M4 仅解码，不混音）。
#[derive(Debug, Clone, PartialEq)]
pub struct AudioData {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

/// 所有导入器产出的 CPU 侧纯数据类型。
#[derive(Debug, Clone, PartialEq)]
pub enum NativeAsset {
    Mesh(MeshData),
    Material(MaterialData),
    Image(ImageData),
    Ktx2(Ktx2Data),
    Audio(AudioData),
    /// 场景快照字节（由 `mir-scene` 版本化编码产生）。
    Scene(Vec<u8>),
}

/// 资产类型标记：关联到导入后的具体纯数据类型。
pub trait Asset: Send + Sync + 'static {
    /// 导入后的具体类型。
    type Native: Clone + Send + Sync + 'static;
    /// 注册表使用的稳定类型标签。
    const TYPE_TAG: &'static str;

    /// 从类型擦除的纯数据容器中借用具体类型。
    fn borrow_native(native: &NativeAsset) -> Option<&Self::Native>;
}

macro_rules! define_asset {
    ($marker:ident, $tag:expr, $variant:ident, $native:ty) => {
        /// 资产类型标记。
        #[derive(Debug, Clone, Copy)]
        pub struct $marker;

        impl Asset for $marker {
            type Native = $native;
            const TYPE_TAG: &'static str = $tag;

            fn borrow_native(native: &NativeAsset) -> Option<&Self::Native> {
                match native {
                    NativeAsset::$variant(value) => Some(value),
                    _ => None,
                }
            }
        }
    };
}

define_asset!(MeshAsset, "mesh", Mesh, MeshData);
define_asset!(MaterialAsset, "material", Material, MaterialData);
define_asset!(ImageAsset, "image", Image, ImageData);
define_asset!(Ktx2Asset, "ktx2", Ktx2, Ktx2Data);
define_asset!(AudioAsset, "audio", Audio, AudioData);
define_asset!(SceneAsset, "scene", Scene, Vec<u8>);

/// 类型化资产句柄。
pub struct Handle<A: Asset> {
    id: AssetId,
    _marker: PhantomData<fn() -> A>,
}

impl<A: Asset> Handle<A> {
    /// 由资产 ID 构造（内部使用）。
    pub(crate) const fn new(id: AssetId) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    /// 资产 ID。
    pub const fn id(self) -> AssetId {
        self.id
    }
}

impl<A: Asset> Clone for Handle<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A: Asset> Copy for Handle<A> {}

impl<A: Asset> PartialEq for Handle<A> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<A: Asset> Eq for Handle<A> {}

impl<A: Asset> std::hash::Hash for Handle<A> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<A: Asset> std::fmt::Debug for Handle<A> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("Handle")
            .field(&self.id)
            .field(&A::TYPE_TAG)
            .finish()
    }
}

/// 资产加载状态。
#[derive(Debug, Clone, PartialEq)]
pub enum AssetState {
    /// 已提交作业，尚未完成。
    Pending,
    /// 加载成功。
    Ready(NativeAsset),
    /// 加载失败，保留可读错误但不 panic。
    Failed(String),
}

impl AssetState {
    /// 是否已完成（成功或失败）。
    pub fn is_settled(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}
