import { externalHelper } from "./dep";

export interface GreeterLike {
  greet(name: string): string;
}

export type Greeting = string;

export enum Mode {
  Loud,
}

export function helper(name: string): string {
  return name.toUpperCase();
}

export function caller(name: string): string {
  return helper(name);
}

export class Greeter {
  greet(name: string): string {
    return caller(name);
  }
}
