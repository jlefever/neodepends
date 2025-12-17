; Class definitions
(
  (class_definition
    name: (identifier) @name) @tag.Class
)

; Method definitions (functions inside classes, including __init__)
; These patterns MUST come before module-level function patterns
(
  (class_definition
    body: (block
      (function_definition
        name: (identifier) @name) @tag.Method
    )
  )
)

; Decorated methods inside classes (e.g., @property, @classmethod, @staticmethod)
(
  (class_definition
    body: (block
      (decorated_definition
        (function_definition
          name: (identifier) @name)) @tag.Method
    )
  )
)

; Nested functions (functions inside other functions)
(
  (function_definition
    body: (block
      (function_definition
        name: (identifier) @name) @tag.Function
    )
  )
)

; Nested decorated functions
(
  (function_definition
    body: (block
      (decorated_definition
        (function_definition
          name: (identifier) @name)) @tag.Function
    )
  )
)

; Module-level function definitions (including async)
(
  (module
    (function_definition
      name: (identifier) @name) @tag.Function
  )
)

; Decorated function definitions at module level
(
  (module
    (decorated_definition
      (function_definition
        name: (identifier) @name)) @tag.Function
  )
)

; Fields matching, use conditional to match both the single statement and multi statements cases
; only match for attributes of a class initiated in "__init__"
(
  (class_definition
    body: (block
      (function_definition
        name: (identifier) @func_name
        body: (block
          [
            (expression_statement
              (assignment
                left: (attribute
                  object: (identifier) @self_ref
                  attribute: (identifier) @name))) @tag.Field
            (assignment
              left: (attribute
                object: (identifier) @self_ref
                attribute: (identifier) @name)) @tag.Field
          ]))))
  (#eq? @func_name "__init__")
  (#eq? @self_ref "self")
)

; Like the above but for case like self.x += 3 instead of self.x = 3
(
  (class_definition
    body: (block
      (function_definition
        name: (identifier) @func_name
        body: (block
          [
            (expression_statement
              (augmented_assignment
                left: (attribute
                  object: (identifier) @self_ref
                  attribute: (identifier) @name))) @tag.Field
            (augmented_assignment
              left: (attribute
                object: (identifier) @self_ref
                attribute: (identifier) @name)) @tag.Field
          ]))))
  (#eq? @func_name "__init__")
  (#eq? @self_ref "self")
)