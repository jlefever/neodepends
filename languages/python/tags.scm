; Class definitions
(
  (comment)? @comment
  .
  (class_definition
    name: (identifier) @name) @tag.Class
)

; Decorated class definitions
(
  (comment)? @comment
  .
  (decorated_definition
    (class_definition
      name: (identifier) @name)) @tag.Class
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

; Module-level function definitions with comment
(
  (module
    (comment)? @comment
    .
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

; Decorated function definitions at module level with comment
(
  (module
    (comment)? @comment
    .
    (decorated_definition
      (function_definition
        name: (identifier) @name)) @tag.Function
  )
)