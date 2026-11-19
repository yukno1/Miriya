//! Inspector 的 UI 无关字段描述模型。
//!
//! 字段元数据只描述数据种类与只读性；控件生成、布局与事件由 UI 后端决定。

/// 字段的数据种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorFieldKind {
    Text,
    Integer,
    Float,
    Boolean,
    Vector3,
    Quaternion,
    Enumeration,
}

/// 一个可编辑字段的描述。
#[derive(Debug, Clone, PartialEq)]
pub struct InspectorField {
    pub name: String,
    pub kind: InspectorFieldKind,
    pub read_only: bool,
}

impl InspectorField {
    /// 创建可编辑字段描述。
    pub fn new(name: impl Into<String>, kind: InspectorFieldKind) -> Self {
        Self {
            name: name.into(),
            kind,
            read_only: false,
        }
    }
}

/// 当前选中对象的字段集合。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InspectorModel {
    fields: Vec<InspectorField>,
}

impl InspectorModel {
    /// 创建空模型。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前字段。
    pub fn fields(&self) -> &[InspectorField] {
        &self.fields
    }

    /// 替换字段描述（由核心在选择变化时更新）。
    pub fn set_fields(&mut self, fields: impl IntoIterator<Item = InspectorField>) {
        self.fields = fields.into_iter().collect();
    }

    /// 清空字段描述。
    pub fn clear(&mut self) {
        self.fields.clear();
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspector_fields_can_be_replaced() {
        let mut model = InspectorModel::new();
        assert!(model.is_empty());
        model.set_fields([InspectorField::new("Name", InspectorFieldKind::Text)]);
        assert_eq!(model.fields().len(), 1);
        assert_eq!(model.fields()[0].name, "Name");
    }
}
