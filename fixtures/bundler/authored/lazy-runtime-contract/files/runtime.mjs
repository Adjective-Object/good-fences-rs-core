// Authored portable contract, not an adaptation of an application implementation.
export class LazyModule {
  constructor(load, options) {
    this.load = load;
    this.options = options;
    this.loaded = false;
  }
  getValue() {
    if (!this.pending) {
      this.pending = this.load().then(namespace => {
        this.loaded = true;
        this.options?.initializer?.(namespace);
        return namespace;
      });
    }
    return this.pending;
  }
}
export class LazyImport {
  constructor(module, getter, govern, extra) {
    this.module = module;
    this.getter = getter;
    this.govern = govern;
    this.extra = extra;
    this.selections = 0;
  }
  async getValue() {
    this.govern?.();
    if (!this.selected) {
      this.selections++;
      this.selected = this.getter(await this.module.getValue());
    }
    return this.selected;
  }
}
export class LazyAction {
  constructor(module, getter, options) {
    this.module = module;
    this.getter = getter;
    this.options = options;
    this.calls = 0;
  }
  async execute(...args) {
    this.calls++;
    const action = this.getter(await this.module.getValue());
    return action(...args);
  }
}
