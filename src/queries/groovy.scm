; Minimal Groovy highlighting (Jenkinsfile, Gradle scripts).
[(line_comment) (block_comment)] @comment
[(string_literal) (character_literal)] @string
(string_interpolation) @embedded
[(decimal_integer_literal) (hex_integer_literal) (octal_integer_literal) (binary_integer_literal)
 (decimal_floating_point_literal) (hex_floating_point_literal)] @number
[(true) (false)] @boolean
(null_literal) @constant.builtin
(this) @variable.builtin
(type_identifier) @type
(juxt_function_call name: (identifier) @function)
(method_invocation name: (identifier) @function.method)
(method_declaration name: (identifier) @function)
["def" "if" "else" "for" "while" "do" "return" "try" "catch" "finally" "throw" "new" "import"
 "class" "static" "switch" "case" "default" "break" "continue" "in" "package"] @keyword
