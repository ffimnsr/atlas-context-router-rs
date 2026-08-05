(preproc_include) @atlas.import

(namespace_definition) @atlas.definition.module

(class_specifier) @atlas.definition.class

(struct_specifier) @atlas.definition.struct

(enum_specifier) @atlas.definition.enum

(class_specifier
  body: (field_declaration_list
    (function_definition) @atlas.definition.method))

(struct_specifier
  body: (field_declaration_list
    (function_definition) @atlas.definition.method))

(function_definition) @atlas.definition.function

(call_expression) @atlas.call
