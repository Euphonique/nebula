; Patterns are written so that no two capture the same node.
(comment) @comment
(doctype) @processing
(erroneous_end_tag_name) @invalid
(tag_name) @tag.name
(entity) @value.entity
(attribute_name) @tag.attribute.name

((attribute
   (attribute_name) @tag.attribute.name
   (quoted_attribute_value (attribute_value) @tag.attribute.value.link))
 (#match? @tag.attribute.name "^(?i:href|src|action|srcset|poster|data)$"))
((attribute
   (attribute_name) @tag.attribute.name
   (quoted_attribute_value (attribute_value) @tag.attribute.value))
 (#not-match? @tag.attribute.name "^(?i:href|src|action|srcset|poster|data)$"))
((attribute
   (attribute_name) @tag.attribute.name
   (attribute_value) @tag.attribute.value.link)
 (#match? @tag.attribute.name "^(?i:href|src|action|srcset|poster|data)$"))
((attribute
   (attribute_name) @tag.attribute.name
   (attribute_value) @tag.attribute.value)
 (#not-match? @tag.attribute.name "^(?i:href|src|action|srcset|poster|data)$"))

(quoted_attribute_value ["\"" "'"] @tag.attribute.value.delimiter)
(attribute "=" @tag.attribute.operator)
["<" ">" "</" "/>"] @tag
