(annotation_type_declaration
  name: (identifier) @name) @tag.annotation

(constructor_declaration
  name: (identifier) @name) @tag.constructor

(class_declaration
  name: (identifier) @name) @tag.class

(enum_declaration
  name: (identifier) @name) @tag.enum

(field_declaration
  declarator: (variable_declarator
    name: (identifier) @name)) @tag.field

(interface_declaration
  name: (identifier) @name) @tag.interface

(method_declaration
  name: (identifier) @name) @tag.method

(record_declaration
  name: (identifier) @name) @tag.record