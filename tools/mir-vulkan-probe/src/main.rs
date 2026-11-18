//! M0 风险验证 PoC：验证 vulkanalia 0.35 + winit 0.30 + raw-window-handle 0.6
//! 能否在本机完成 Vulkan 1.3 实例 → 物理设备 → 逻辑设备 → surface → 交换链前置查询
//! 的最小链路。
//!
//! 这是 spike，不进入运行时 crate；验证过的代码模式将在 M2 搬进 `mir-render`。
//! 运行：`cargo run -p mir-vulkan-probe`（会短暂打开一个窗口）。

use std::ffi::{CStr, c_char};

use vulkanalia::Entry;
use vulkanalia::loader::{LIBRARY, LibloadingLoader};
use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::KhrSurfaceExtensionInstanceCommands;
use vulkanalia::window as vulkan_window;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent as WinitWindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{WindowAttributes, WindowId};

/// 探测结果，用于形成风险结论。
#[derive(Debug, Default)]
struct ProbeReport {
    loader_loaded: bool,
    loader_version: String,
    instance_created: bool,
    surface_created: bool,
    physical_device: String,
    vulkan_1_3: bool,
    graphics_queue: bool,
    present_queue: bool,
    surface_format: String,
    present_mode: String,
}

impl ProbeReport {
    fn print(&self) {
        println!("\n========== Vulkan PoC 风险结论 ==========");
        println!("Loader 加载:        {}", yes(self.loader_loaded));
        println!("Loader 版本:        {}", self.loader_version);
        println!("实例创建:           {}", yes(self.instance_created));
        println!("Surface 创建:       {}", yes(self.surface_created));
        println!("物理设备:           {}", self.physical_device);
        println!("支持 Vulkan 1.3:    {}", yes(self.vulkan_1_3));
        println!("图形队列:           {}", yes(self.graphics_queue));
        println!("呈现队列:           {}", yes(self.present_queue));
        println!("Surface 格式:       {}", self.surface_format);
        println!("呈现模式:           {}", self.present_mode);
        println!("=========================================");
    }
}

fn yes(value: bool) -> &'static str {
    if value { "是" } else { "否" }
}

struct ProbeApp {
    report: ProbeReport,
    error: Option<String>,
}

impl ApplicationHandler for ProbeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.probe(event_loop) {
            self.error = Some(error);
        }
        event_loop.exit();
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _id: WindowId,
        _event: WinitWindowEvent,
    ) {
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {}
}

impl ProbeApp {
    fn probe(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        // 1) 加载 Vulkan loader（vulkan-1.dll）
        // SAFETY: 从系统路径加载标准 Vulkan loader，生命周期覆盖整个探测过程。
        let loader = unsafe { LibloadingLoader::new(LIBRARY) }
            .map_err(|e| format!("加载 {LIBRARY} 失败: {e}"))?;
        let entry =
            unsafe { Entry::new(loader) }.map_err(|e| format!("初始化 Vulkan Entry 失败: {e}"))?;
        let version = entry
            .version()
            .map_err(|e| format!("查询 loader 版本失败: {e}"))?;
        self.report.loader_loaded = true;
        self.report.loader_version =
            format!("{}.{}.{}", version.major, version.minor, version.patch);

        // 2) 创建窗口，拿到 raw-window-handle
        let window = event_loop
            .create_window(WindowAttributes::default().with_title("Miriya Vulkan PoC"))
            .map_err(|e| format!("创建窗口失败: {e}"))?;

        // 3) 查询窗口所需实例扩展（KHR_surface + KHR_win32_surface）
        let required: Vec<*const c_char> = vulkan_window::get_required_instance_extensions(&window)
            .iter()
            .map(|ext| ext.as_ptr())
            .collect();

        // 4) 创建 Instance（Vulkan 1.3）
        let app_info = vk::ApplicationInfo::builder()
            .application_name(b"Miriya Vulkan PoC\0")
            .application_version(vk::make_version(1, 0, 0))
            .engine_name(b"Miriya\0")
            .engine_version(vk::make_version(0, 1, 0))
            .api_version(vk::make_version(1, 3, 0))
            .build();
        let info = vk::InstanceCreateInfo::builder()
            .application_info(&app_info)
            .enabled_extension_names(&required);
        let instance = unsafe { entry.create_instance(&info, None) }
            .map_err(|e| format!("创建 Instance 失败: {e}"))?;
        self.report.instance_created = true;

        // 5) 创建 surface（验证 rwh 0.6 桥接）
        let surface = unsafe { vulkan_window::create_surface(&instance, &window, &window) }
            .map_err(|e| format!("创建 surface 失败: {e}"))?;
        self.report.surface_created = true;

        // 6) 枚举物理设备，选第一个支持图形队列 + present 的设备
        // SAFETY: instance 有效且在整个探测期间存活。
        let devices = unsafe { instance.enumerate_physical_devices() }
            .map_err(|e| format!("枚举物理设备失败: {e}"))?;
        let mut chosen = None;
        for device in devices {
            // SAFETY: device 来自本 instance 的有效枚举结果。
            let props = unsafe { instance.get_physical_device_properties(device) };
            let name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }
                .to_string_lossy()
                .to_string();
            let api_ok = props.api_version >= vk::make_version(1, 3, 0);
            // SAFETY: device 有效。
            let queue_families =
                unsafe { instance.get_physical_device_queue_family_properties(device) };
            let graphics = queue_families
                .iter()
                .position(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS));
            let present = queue_families.iter().enumerate().find_map(|(i, _)| {
                let supported = unsafe {
                    instance.get_physical_device_surface_support_khr(device, i as u32, surface)
                };
                supported.unwrap_or(false).then_some(i)
            });
            if let (Some(g), Some(p)) = (graphics, present) {
                self.report.physical_device = name;
                self.report.vulkan_1_3 = api_ok;
                self.report.graphics_queue = true;
                self.report.present_queue = true;
                chosen = Some((device, g as u32, p as u32));
                break;
            }
        }
        let (physical_device, graphics_index, present_index) =
            chosen.ok_or_else(|| "没有找到同时支持图形与呈现队列的物理设备".to_string())?;

        // 7) 查询 surface 格式与呈现模式（交换链前置条件）
        let formats =
            unsafe { instance.get_physical_device_surface_formats_khr(physical_device, surface) }
                .map_err(|e| format!("查询 surface 格式失败: {e}"))?;
        if let Some(format) = formats.first() {
            self.report.surface_format = format!("{:?}", format.format);
        }
        let present_modes = unsafe {
            instance.get_physical_device_surface_present_modes_khr(physical_device, surface)
        }
        .map_err(|e| format!("查询呈现模式失败: {e}"))?;
        if let Some(mode) = present_modes.first() {
            self.report.present_mode = format!("{mode:?}");
        }

        // 8) 创建逻辑设备（验证 device extension + 队列创建）
        let queue_priorities = [1.0];
        let mut queue_infos = vec![
            vk::DeviceQueueCreateInfo::builder()
                .queue_family_index(graphics_index)
                .queue_priorities(&queue_priorities)
                .build(),
        ];
        if present_index != graphics_index {
            queue_infos.push(
                vk::DeviceQueueCreateInfo::builder()
                    .queue_family_index(present_index)
                    .queue_priorities(&queue_priorities)
                    .build(),
            );
        }
        let device_extensions = [vk::KHR_SWAPCHAIN_EXTENSION.name.as_ptr()];
        let device_info = vk::DeviceCreateInfo::builder()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&device_extensions);
        let device = unsafe { instance.create_device(physical_device, &device_info, None) }
            .map_err(|e| format!("创建逻辑设备失败: {e}"))?;

        // 清理：surface → device → instance；窗口由 winit 析构
        unsafe {
            instance.destroy_surface_khr(surface, None);
            device.destroy_device(None);
            instance.destroy_instance(None);
        }

        let _ = window;
        Ok(())
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("创建事件循环失败");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = ProbeApp {
        report: ProbeReport::default(),
        error: None,
    };
    event_loop.run_app(&mut app).expect("PoC 事件循环异常退出");
    app.report.print();
    if let Some(error) = &app.error {
        eprintln!("\nPoC 未完成全部探测: {error}");
        eprintln!("M0 结论：API 编译链路已验证；运行时设备可用性取决于本机驱动。");
        std::process::exit(2);
    }
}
