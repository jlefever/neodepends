(annotation_type_declaration
  name: (identifier) @name) @tag.Annotation

(constructor_declaration
  name: (identifier) @name) @tag.Constructor

(class_declaration
  name: (identifier) @name) @tag.Class

(enum_declaration
  name: (identifier) @name) @tag.Enum

(field_declaration
  declarator: (variable_declarator
    name: (identifier) @name)) @tag.Field

(interface_declaration
  name: (identifier) @name) @tag.Interface

(method_declaration
  name: (identifier) @name) @tag.Method

(record_declaration
  name: (identifier) @name) @tag.Record