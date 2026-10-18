; Patterns are written so that no two capture the same node.
(comment) @comment
(js_comment) @comment

((property_name) @identifier.variable (#match? @identifier.variable "^--"))
((property_name) @style.attribute.name (#not-match? @style.attribute.name "^--"))
((plain_value) @identifier.variable (#match? @identifier.variable "^--"))
((plain_value) @style.value.keyword (#not-match? @style.value.keyword "^--"))

(pseudo_class_selector ":" @style.selector.pseudoclass.prefix)
(pseudo_class_selector (class_name) @style.selector.pseudoclass)
(pseudo_element_selector "::" @style.selector.pseudoelement.prefix)
(pseudo_element_selector (tag_name) @style.selector.pseudoelement)
(class_selector "." @style.selector.class.prefix)
(class_selector (class_name) @style.selector.class)
(id_selector "#" @style.selector.id.prefix)
(id_selector (id_name) @style.selector.id)
(descendant_selector (tag_name) @style.selector.element)
(child_selector (tag_name) @style.selector.element)
(sibling_selector (tag_name) @style.selector.element)
(adjacent_sibling_selector (tag_name) @style.selector.element)
(selectors (tag_name) @style.selector.element)
(class_selector (tag_name) @style.selector.element)
(id_selector (tag_name) @style.selector.element)
(pseudo_class_selector (tag_name) @style.selector.element)
(attribute_selector (tag_name) @style.selector.element)
(nesting_selector) @style.selector.nesting
(universal_selector) @style.selector.universal
(attribute_name) @tag.attribute.name
(namespace_name) @identifier.type
(keyframes_name) @identifier.type
(feature_name) @style.attribute.name

(color_value) @style.value.color.hex
(unit) @style.value.number.unit
(integer_value) @value.number
(float_value) @value.number
(string_value) @string
(function_name) @identifier.function
(important) @keyword

(at_keyword) @style.at
["@media" "@import" "@charset" "@namespace" "@supports" "@keyframes"] @style.at
["and" "or" "not" "only"] @keyword
(to) @keyword
(from) @keyword

["{" "}" "(" ")"] @bracket
(declaration ":" @operator)
(feature_query ":" @operator)
[";" ","] @operator
["~" ">" "+" "-" "*" "/" "=" "^=" "|=" "~=" "$=" "*="] @operator
