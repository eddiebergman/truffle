"use strict";
// A small DOM that tolerates the top-level wiring of every static script.
// Lookups by selector create the element on first use, so a test can load a
// whole script and then read the handlers that the script attached.

class FakeClassList {
  constructor(element) {
    this.element = element;
  }
  get names() {
    return String(this.element.className || "")
      .split(/\s+/)
      .filter(Boolean);
  }
  contains(name) {
    return this.names.includes(name);
  }
  add(...names) {
    this.element.className = [...new Set([...this.names, ...names])].join(" ");
  }
  remove(...names) {
    this.element.className = this.names
      .filter((name) => !names.includes(name))
      .join(" ");
  }
  toggle(name, force) {
    const on = force === undefined ? !this.contains(name) : Boolean(force);
    if (on) {
      this.add(name);
    } else {
      this.remove(name);
    }
    return on;
  }
  replace(from, to) {
    if (!this.contains(from)) {
      return false;
    }
    this.remove(from);
    this.add(to);
    return true;
  }
}

class FakeEventTarget {
  constructor() {
    this.listeners = {};
  }
  addEventListener(type, listener) {
    this.listeners[type] ??= [];
    this.listeners[type].push(listener);
  }
  removeEventListener(type, listener) {
    this.listeners[type] = (this.listeners[type] || []).filter(
      (item) => item !== listener,
    );
  }
  dispatchEvent(event) {
    for (const listener of this.listeners[event.type] || []) {
      listener.call(this, event);
    }
    this[`on${event.type}`]?.call(this, event);
    return true;
  }
}

class FakeElement extends FakeEventTarget {
  constructor(tagName = "div", ownerDocument = null) {
    super();
    Object.assign(this, {
      tagName: String(tagName).toUpperCase(),
      ownerDocument,
      id: "",
      className: "",
      textContent: "",
      innerHTML: "",
      value: "",
      hidden: false,
      disabled: false,
      checked: false,
      open: false,
      children: [],
      parentElement: null,
      dataset: {},
      style: { setProperty() {}, removeProperty() {} },
      attributes: {},
      options: [],
      selected: new Map(),
    });
    this.classList = new FakeClassList(this);
    const named = new Map();
    // Form controls by name, created on first use like selector lookups.
    this.elements = new Proxy(
      {},
      {
        get: (_, name) => {
          if (typeof name !== "string") {
            return undefined;
          }
          if (!named.has(name)) {
            named.set(name, new FakeElement("input", ownerDocument));
          }
          return named.get(name);
        },
      },
    );
  }
  get isConnected() {
    return Boolean(this.parentElement?.isConnected);
  }
  get firstChild() {
    return this.children[0] || null;
  }
  get lastChild() {
    return this.children.at(-1) || null;
  }
  get firstElementChild() {
    return this.children[0] || null;
  }
  get lastElementChild() {
    return this.children.at(-1) || null;
  }
  get childElementCount() {
    return this.children.length;
  }
  setAttribute(name, value) {
    this.attributes[name] = String(value);
    if (name === "id") {
      this.id = String(value);
    }
  }
  getAttribute(name) {
    return Object.hasOwn(this.attributes, name) ? this.attributes[name] : null;
  }
  hasAttribute(name) {
    return Object.hasOwn(this.attributes, name);
  }
  removeAttribute(name) {
    delete this.attributes[name];
  }
  toggleAttribute(name, force) {
    const on = force === undefined ? !this.hasAttribute(name) : Boolean(force);
    if (on) {
      this.setAttribute(name, "");
    } else {
      this.removeAttribute(name);
    }
    return on;
  }
  append(...nodes) {
    for (const node of nodes) {
      if (node instanceof FakeElement) {
        node.remove();
        node.parentElement = this;
      }
      this.children.push(node);
    }
  }
  appendChild(node) {
    this.append(node);
    return node;
  }
  prepend(...nodes) {
    const rest = this.children;
    this.children = [];
    this.append(...nodes);
    this.children.push(...rest);
  }
  replaceChildren(...nodes) {
    for (const child of this.children) {
      if (child instanceof FakeElement) {
        child.parentElement = null;
      }
    }
    this.children = [];
    this.append(...nodes);
  }
  before(...nodes) {
    this.parentElement?.insertBefore(nodes, this);
  }
  after(...nodes) {
    this.parentElement?.insertBefore(nodes, this, 1);
  }
  insertBefore(inserted, reference, offset = 0) {
    const nodes = [inserted].flat();
    for (const node of nodes) {
      if (node instanceof FakeElement) {
        node.remove();
        node.parentElement = this;
      }
    }
    const index = this.children.indexOf(reference);
    this.children.splice(
      index < 0 ? this.children.length : index + offset,
      0,
      ...nodes,
    );
    return nodes[0];
  }
  remove() {
    const parent = this.parentElement;
    if (parent) {
      parent.children = parent.children.filter((child) => child !== this);
    }
    this.parentElement = null;
  }
  add(option) {
    this.options.push(option);
  }
  contains(node) {
    return (
      node === this ||
      this.children.some(
        (child) => child instanceof FakeElement && child.contains(node),
      )
    );
  }
  // Ancestors are created on first use too, one per selector.
  closest(selector) {
    this.ancestors ??= new Map();
    if (!this.ancestors.has(selector)) {
      this.ancestors.set(selector, new FakeElement("div", this.ownerDocument));
    }
    return this.ancestors.get(selector);
  }
  matches() {
    return false;
  }
  querySelector(selector) {
    return this.ownerDocument.querySelector(selector);
  }
  querySelectorAll() {
    return [];
  }
  focus() {
    if (this.ownerDocument) {
      this.ownerDocument.activeElement = this;
    }
  }
  blur() {}
  click() {
    this.dispatchEvent({ type: "click", target: this });
  }
  showModal() {
    this.open = true;
  }
  show() {
    this.open = true;
  }
  close() {
    this.open = false;
  }
  select() {}
  scrollIntoView() {}
  getBoundingClientRect() {
    return { top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0 };
  }
  cloneNode() {
    return new FakeElement(this.tagName, this.ownerDocument);
  }
}

class FakeDocument extends FakeEventTarget {
  constructor() {
    super();
    this.elements = new Map();
    this.hidden = false;
    this.visibilityState = "visible";
    this.title = "";
    this.documentElement = new FakeElement("html", this);
    this.body = new FakeElement("body", this);
    this.head = new FakeElement("head", this);
    this.activeElement = this.body;
  }
  createElement(tagName) {
    return new FakeElement(tagName, this);
  }
  createElementNS(_, tagName) {
    return new FakeElement(tagName, this);
  }
  createTextNode(text) {
    return { nodeType: 3, textContent: String(text) };
  }
  createDocumentFragment() {
    return new FakeElement("#fragment", this);
  }
  // The same selector always returns the same element.
  querySelector(selector) {
    if (!this.elements.has(selector)) {
      const element = new FakeElement("div", this);
      if (selector.startsWith("#")) {
        element.id = selector.slice(1);
      }
      this.elements.set(selector, element);
    }
    return this.elements.get(selector);
  }
  querySelectorAll() {
    return [];
  }
  getElementById(id) {
    return this.querySelector(`#${id}`);
  }
}

class FakeWindow extends FakeEventTarget {
  constructor(document) {
    super();
    Object.assign(this, {
      document,
      location: {
        hash: "",
        pathname: "/",
        search: "",
        href: "http://localhost/",
        origin: "http://localhost",
      },
      navigator: { platform: "", userAgent: "", language: "en" },
      history: { state: null, pushState() {}, replaceState() {} },
      innerWidth: 1280,
      innerHeight: 800,
    });
  }
  matchMedia() {
    return { matches: false, addEventListener() {}, removeEventListener() {} };
  }
  getComputedStyle() {
    return { getPropertyValue: () => "" };
  }
}

class FakeStorage {
  constructor() {
    this.values = new Map();
  }
  getItem(key) {
    return this.values.has(key) ? this.values.get(key) : null;
  }
  setItem(key, value) {
    this.values.set(key, String(value));
  }
  removeItem(key) {
    this.values.delete(key);
  }
}

class InertObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}

// Browser globals for running scripts at load time without real side effects:
// timers never fire and requests never settle.
function browserHost() {
  const document = new FakeDocument(),
    window = new FakeWindow(document);
  const host = {
    document,
    window,
    localStorage: new FakeStorage(),
    sessionStorage: new FakeStorage(),
    location: window.location,
    navigator: window.navigator,
    history: window.history,
    matchMedia: () => window.matchMedia(),
    getComputedStyle: () => window.getComputedStyle(),
    setTimeout: () => 0,
    setInterval: () => 0,
    clearTimeout() {},
    clearInterval() {},
    requestAnimationFrame: () => 0,
    cancelAnimationFrame() {},
    requestIdleCallback: () => 0,
    fetch: () => new Promise(() => {}),
    Node: { ELEMENT_NODE: 1, TEXT_NODE: 3 },
    HTMLElement: FakeElement,
    MutationObserver: InertObserver,
    ResizeObserver: InertObserver,
    IntersectionObserver: InertObserver,
    Option: class {
      constructor(text = "", value = text) {
        this.text = text;
        this.value = value;
      }
    },
    Event: class {
      constructor(type, init = {}) {
        this.type = type;
        Object.assign(this, init);
      }
    },
    CustomEvent: class {
      constructor(type, init = {}) {
        this.type = type;
        this.detail = init.detail;
      }
    },
    addEventListener: (...args) => window.addEventListener(...args),
    removeEventListener: (...args) => window.removeEventListener(...args),
    dispatchEvent: (event) => window.dispatchEvent(event),
    console: { log() {}, info() {}, warn() {}, error() {}, debug() {} },
  };
  for (const [name, value] of Object.entries(host)) {
    if (!(name in window)) {
      window[name] = value;
    }
  }
  window.window = window;
  return host;
}

module.exports = {
  FakeElement,
  FakeDocument,
  FakeWindow,
  FakeStorage,
  browserHost,
};
