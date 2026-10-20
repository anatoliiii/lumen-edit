; Minimal HCL highlighting (Terraform, Nomad, Packer).
(comment) @comment
[(string_lit) (quoted_template) (template_literal)] @string
(template_interpolation) @embedded
(numeric_lit) @number
(bool_lit) @boolean
(null_lit) @constant.builtin
(block (identifier) @keyword)
(attribute (identifier) @property)
(function_call (identifier) @function)
(variable_expr (identifier) @variable)
["if" "else" "for" "in" "endif" "endfor"] @keyword
