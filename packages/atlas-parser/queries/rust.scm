(function_item
  name: (identifier) @atlas.name) @atlas.definition.function

(function_signature_item
  name: (identifier) @atlas.name) @atlas.definition.function_signature

(mod_item
  name: (identifier) @atlas.name) @atlas.definition.module

(struct_item
  name: (type_identifier) @atlas.name) @atlas.definition.struct

(enum_item
  name: (type_identifier) @atlas.name) @atlas.definition.enum

(trait_item
  name: (type_identifier) @atlas.name) @atlas.definition.trait

(const_item
  name: (identifier) @atlas.name) @atlas.definition.const

(static_item
  name: (identifier) @atlas.name) @atlas.definition.static

(impl_item
  type: (_type) @atlas.impl.type) @atlas.definition.impl

(impl_item
  trait: (_) @atlas.impl.trait) @atlas.impl.item

(call_expression
  function: (_) @atlas.call.target) @atlas.call

(call_expression
  function: (field_expression
    value: (_) @atlas.call.receiver
    field: (field_identifier) @atlas.call.method)) @atlas.call

(call_expression
  function: (generic_function
    function: (field_expression
      value: (_) @atlas.call.receiver
      field: (field_identifier) @atlas.call.method))) @atlas.call

(use_declaration
  argument: (_) @atlas.reference.use_argument) @atlas.reference.use

(type_identifier) @atlas.reference.type

(scoped_type_identifier) @atlas.reference.type
