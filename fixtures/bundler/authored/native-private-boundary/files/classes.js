const Base = class {};
export const value = class Self extends (() => Base)() {
  static own() {
    return Self;
  }
};

export function read() {
  return [value.name, value.own() === value, Object.getPrototypeOf(value) === Base];
}
