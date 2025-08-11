(
  (block_comment)? @comment
  .
  (annotation_type_declaration
    name: (identifier) @name) @tag.Annotation
)

(
  (block_comment)? @comment
  .
  (constructor_declaration
    name: (identifier) @name) @tag.Constructor
)

(
  (block_comment)? @comment
  .
  (class_declaration
    name: (identifier) @name) @tag.Class
)

(
  (block_comment)? @comment
  .
  (enum_declaration
    name: (identifier) @name) @tag.Enum
)

(
  (block_comment)? @comment
  .
  (field_declaration
    declarator: (variable_declarator
      name: (identifier) @name)) @tag.Field
)

(
  (block_comment)? @comment
  .
  (interface_declaration
    name: (identifier) @name) @tag.Interface
)

(
  (block_comment)? @comment
  .
  (method_declaration
    name: (identifier) @name) @tag.Method
)

(
  (block_comment)? @comment
  .
  (record_declaration
    name: (identifier) @name) @tag.Record
)
