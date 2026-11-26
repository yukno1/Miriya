//! Khronos 校验层与 debug utils 回调。

use std::ffi::{CStr, c_char, c_void};

use vulkanalia::prelude::v1_0::*;
use vulkanalia::vk::{EntryV1_0, ExtDebugUtilsExtensionInstanceCommands};
use vulkanalia::{Entry, Instance};

pub(super) const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// 查询 loader 是否提供 Khronos 校验层；缺失时调用方降级关闭而不是报错。
pub(super) fn validation_layer_available(entry: &Entry) -> bool {
    // SAFETY: `entry` 由当前进程加载的 Vulkan loader 构造，查询不依赖实例。
    unsafe { entry.enumerate_instance_layer_properties() }
        .map(|layers| {
            layers
                .iter()
                .any(|layer| layer.layer_name.as_cstr() == VALIDATION_LAYER)
        })
        .unwrap_or(false)
}

/// debug utils 回调信息：错误/警告记入 `mir-log`，并带上类型、ID 与对象名。
fn debug_messenger_create_info() -> vk::DebugUtilsMessengerCreateInfoEXT {
    vk::DebugUtilsMessengerCreateInfoEXT::builder()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .user_callback(Some(debug_callback))
        .build()
}

/// 安全读取校验层回调中的 C 字符串；调用方需保证指针有效或以空指针表示缺省。
unsafe fn debug_cstr(ptr: *const c_char, fallback: &str) -> String {
    if ptr.is_null() {
        fallback.to_string()
    } else {
        // SAFETY: 调用方保证 `ptr` 为空指针或有效的 NUL 结尾字符串。
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

unsafe extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_types: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _user_data: *mut c_void,
) -> vk::Bool32 {
    if data.is_null() {
        return vk::FALSE;
    }
    // SAFETY: 回调数据由 Vulkan 校验层提供，调用期间有效。
    let data = unsafe { &*data };
    let id = unsafe { debug_cstr(data.message_id_name, "unknown") };
    let message = unsafe { debug_cstr(data.message, "") };
    let mut object_names = Vec::new();
    if !data.objects.is_null() && data.object_count > 0 {
        // SAFETY: `object_count` 与 `objects` 由校验层保证一致且调用期间有效。
        let objects =
            unsafe { std::slice::from_raw_parts(data.objects, data.object_count as usize) };
        for object in objects.iter().take(4) {
            let name = unsafe { debug_cstr(object.object_name, "unnamed") };
            object_names.push(format!(
                "{:?}:{}:{}",
                object.object_type, object.object_handle, name
            ));
        }
        if data.object_count > 4 {
            object_names.push(format!("+{}", data.object_count - 4));
        }
    }
    let objects = if object_names.is_empty() {
        "-".to_string()
    } else {
        object_names.join(", ")
    };
    let payload = format!("[{message_types:?}/{id}] objects=[{objects}] {message}");

    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        mir_log::error!(target: "vulkan", "{payload}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        mir_log::warn!(target: "vulkan", "{payload}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) {
        mir_log::info!(target: "vulkan", "{payload}");
    } else {
        mir_log::debug!(target: "vulkan", "{payload}");
    }

    // 返回 FALSE：校验层不应因为日志而中止 Vulkan 调用。
    vk::FALSE
}

pub(super) fn create_debug_messenger(instance: &Instance) -> Option<vk::DebugUtilsMessengerEXT> {
    let create_info = debug_messenger_create_info();
    // SAFETY: instance 有效且创建时启用了 VK_EXT_debug_utils；回调为 `extern "system"` 且不逃逸数据。
    match unsafe { instance.create_debug_utils_messenger_ext(&create_info, None) } {
        Ok(messenger) => {
            mir_log::info!(target: "vulkan", "校验层已启用");
            Some(messenger)
        }
        Err(error) => {
            mir_log::warn!(target: "vulkan", "debug utils messenger 创建失败，校验层不可用：{error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_layer_name_is_standard() {
        assert_eq!(VALIDATION_LAYER.to_bytes(), b"VK_LAYER_KHRONOS_validation");
    }

    #[test]
    fn debug_messenger_subscribes_to_errors_and_warnings() {
        let info = debug_messenger_create_info();
        assert!(info.message_severity.contains(
            vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
        ));
        assert!(
            info.message_type
                .contains(vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION)
        );
        assert!(info.user_callback.is_some());
    }
}
