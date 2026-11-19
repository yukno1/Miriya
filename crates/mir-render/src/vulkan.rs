//! `vulkanalia` 内部实现：实例、物理设备、逻辑设备与队列。
//!
//! 本模块是 crate 私有的；Vulkan 类型不得出现在 `mir-render` 的公共 API 中。

use std::ffi::{CStr, c_char};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use vulkanalia::loader::{LIBRARY, LibloadingLoader};
use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::KhrSurfaceExtensionInstanceCommands;
use vulkanalia::window as vulkan_window;
use vulkanalia::{Device, Entry, Instance};

use crate::RenderInitError;

const API_VERSION: u32 = vk::make_version(1, 3, 0);

/// Vulkan 核心对象。按 surface → device → instance 的逆序销毁。
pub(super) struct VulkanCore {
    surface: Option<vk::SurfaceKHR>,
    device: Device,
    instance: Instance,
    // Entry 持有加载器；必须比 Instance / Device 活得更久。
    _entry: Entry,
    _graphics_queue: vk::Queue,
    _present_queue: vk::Queue,
    graphics_family: u32,
    present_family: u32,
    device_name: String,
    api_version: String,
    surface_format: Option<String>,
    present_mode: Option<String>,
}

impl VulkanCore {
    /// 无窗口（headless）初始化：不创建 surface，不启用交换链扩展。
    pub(super) fn new_headless() -> Result<Self, RenderInitError> {
        Self::create(None, None)
    }

    /// 带窗口初始化：创建 surface，并要求物理设备支持呈现。
    pub(super) fn with_surface(
        display: &dyn HasDisplayHandle,
        window: &dyn HasWindowHandle,
    ) -> Result<Self, RenderInitError> {
        Self::create(Some(display), Some(window))
    }

    pub(super) fn device_name(&self) -> &str {
        &self.device_name
    }

    pub(super) fn api_version(&self) -> &str {
        &self.api_version
    }

    pub(super) fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    pub(super) fn queue_families(&self) -> (u32, u32) {
        (self.graphics_family, self.present_family)
    }

    pub(super) fn surface_format(&self) -> Option<&str> {
        self.surface_format.as_deref()
    }

    pub(super) fn present_mode(&self) -> Option<&str> {
        self.present_mode.as_deref()
    }

    fn create(
        display: Option<&dyn HasDisplayHandle>,
        window: Option<&dyn HasWindowHandle>,
    ) -> Result<Self, RenderInitError> {
        // SAFETY: 从系统路径加载标准 Vulkan loader；返回的 Entry 在本函数返回后由 VulkanCore 持有。
        let loader = unsafe { LibloadingLoader::new(LIBRARY) }
            .map_err(|error| RenderInitError::Loader(error.to_string()))?;
        // SAFETY: loader 有效且被 Entry 接管。
        let entry = unsafe { Entry::new(loader) }
            .map_err(|error| RenderInitError::Loader(error.to_string()))?;

        let required_extensions: Vec<*const c_char> = match window {
            Some(window) => vulkan_window::get_required_instance_extensions(window)
                .iter()
                .map(|extension| extension.as_ptr())
                .collect(),
            None => Vec::new(),
        };

        let app_info = vk::ApplicationInfo::builder()
            .application_name(b"Miriya\0")
            .application_version(vk::make_version(0, 1, 0))
            .engine_name(b"Miriya\0")
            .engine_version(vk::make_version(0, 1, 0))
            .api_version(API_VERSION)
            .build();
        let instance_info = vk::InstanceCreateInfo::builder()
            .application_info(&app_info)
            .enabled_extension_names(&required_extensions);
        // SAFETY: app_info / instance_info / required_extensions 在调用期间有效，入口为有效 Vulkan 实例入口。
        let instance = unsafe { entry.create_instance(&instance_info, None) }
            .map_err(|error| RenderInitError::Instance(error.to_string()))?;

        let surface = match (display, window) {
            (Some(display), Some(window)) => {
                // SAFETY: 调用方保证窗口与 display 至少存活到 surface 被销毁；VulkanCore::drop 负责逆序销毁。
                let surface = unsafe { vulkan_window::create_surface(&instance, display, window) }
                    .map_err(|error| RenderInitError::Surface(error.to_string()))?;
                Some(surface)
            }
            _ => None,
        };

        // SAFETY: instance 句柄有效且在本函数内保持存活。
        let devices = unsafe { instance.enumerate_physical_devices() }
            .map_err(|error| RenderInitError::Device(error.to_string()))?;

        let mut chosen = None;
        for device in devices {
            // SAFETY: device 来自本 instance 的有效枚举结果。
            let properties = unsafe { instance.get_physical_device_properties(device) };
            if properties.api_version < API_VERSION {
                continue;
            }
            // SAFETY: properties.device_name 是设备返回的定长 C 字符串。
            let name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            // SAFETY: device 有效。
            let families = unsafe { instance.get_physical_device_queue_family_properties(device) };
            let graphics = families
                .iter()
                .position(|family| family.queue_flags.contains(vk::QueueFlags::GRAPHICS));
            let present = match surface {
                Some(surface) => families.iter().enumerate().find_map(|(index, _)| {
                    // SAFETY: device 与 surface 均由本 instance 创建且仍然有效。
                    unsafe {
                        instance.get_physical_device_surface_support_khr(
                            device,
                            index as u32,
                            surface,
                        )
                    }
                    .ok()
                    .unwrap_or(false)
                    .then_some(index)
                }),
                None => graphics,
            };
            if let (Some(graphics), Some(present)) = (graphics, present) {
                chosen = Some((
                    device,
                    graphics as u32,
                    present as u32,
                    name,
                    properties.api_version,
                ));
                break;
            }
        }
        let (physical_device, graphics_family, present_family, device_name, api_version) =
            chosen.ok_or(RenderInitError::NoDevice)?;

        let priorities = [1.0_f32];
        let mut queue_infos = vec![
            vk::DeviceQueueCreateInfo::builder()
                .queue_family_index(graphics_family)
                .queue_priorities(&priorities)
                .build(),
        ];
        if present_family != graphics_family {
            queue_infos.push(
                vk::DeviceQueueCreateInfo::builder()
                    .queue_family_index(present_family)
                    .queue_priorities(&priorities)
                    .build(),
            );
        }

        let swapchain_name = vk::KHR_SWAPCHAIN_EXTENSION.name;
        let device_extensions: Vec<*const c_char> = if surface.is_some() {
            vec![swapchain_name.as_ptr()]
        } else {
            Vec::new()
        };
        let device_info = vk::DeviceCreateInfo::builder()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&device_extensions);
        // SAFETY: physical_device 支持所选队列族；device_info 及扩展名数组在调用期间有效。
        let device = unsafe { instance.create_device(physical_device, &device_info, None) }
            .map_err(|error| RenderInitError::Device(error.to_string()))?;
        // SAFETY: device 有效，且创建时请求了这两个队列族。
        let graphics_queue = unsafe { device.get_device_queue(graphics_family, 0) };
        let present_queue = unsafe { device.get_device_queue(present_family, 0) };

        let (surface_format, present_mode) = match surface {
            Some(surface) => {
                // SAFETY: physical_device 与 surface 均有效且仍存活。
                let formats = unsafe {
                    instance.get_physical_device_surface_formats_khr(physical_device, surface)
                }
                .map_err(|error| RenderInitError::Surface(error.to_string()))?;
                // SAFETY: physical_device 与 surface 均有效且仍存活。
                let modes = unsafe {
                    instance.get_physical_device_surface_present_modes_khr(physical_device, surface)
                }
                .map_err(|error| RenderInitError::Surface(error.to_string()))?;
                (
                    formats.first().map(|entry| format!("{:?}", entry.format)),
                    modes.first().map(|mode| format!("{mode:?}")),
                )
            }
            None => (None, None),
        };

        Ok(Self {
            surface,
            device,
            instance,
            _entry: entry,
            _graphics_queue: graphics_queue,
            _present_queue: present_queue,
            graphics_family,
            present_family,
            device_name,
            api_version: format_version(api_version),
            surface_format,
            present_mode,
        })
    }
}

impl Drop for VulkanCore {
    fn drop(&mut self) {
        // SAFETY: 本对象仍持有有效的 Vulkan 句柄；按 surface → device → instance 逆序释放。
        unsafe {
            if let Some(surface) = self.surface.take() {
                self.instance.destroy_surface_khr(surface, None);
            }
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

fn format_version(version: u32) -> String {
    format!(
        "{}.{}.{}",
        version >> 22,
        (version >> 12) & 0x3ff,
        version & 0xfff
    )
}
