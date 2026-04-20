import defaultThing, { helperAlias as helperImport } from "./dep.js";

function helper(value) {
  return value + 1;
}

function caller() {
  return helper(1);
}

class Greeter {
  greet() {
    return caller();
  }
}
