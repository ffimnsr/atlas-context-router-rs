(import_statement) @atlas.import

(import_from_statement) @atlas.import

(class_definition) @atlas.definition.class

(decorated_definition
  definition: (class_definition)) @atlas.definition.class

(function_definition) @atlas.definition.function

(decorated_definition
  definition: (function_definition)) @atlas.definition.function

(assignment) @atlas.definition.variable

(augmented_assignment) @atlas.definition.variable

(call) @atlas.call
