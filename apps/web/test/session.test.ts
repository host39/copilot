// Token storage and the native server URL.

import { afterEach, beforeEach, describe, expect, test } from "@rstest/core";
import { clearToken, followStorage, getToken, setToken, subscribeToken } from "@web/auth/token";
import {
  buildServerUrl,
  forgetServerUrl,
  isAndroid,
  isNative,
  parseServerUrl,
  saveServerUrl,
  serverUrl,
} from "@web/config";

/** A Map-backed `localStorage`. */
class MemoryStorage implements Storage {
  private readonly items = new Map<string, string>();
  get length() {
    return this.items.size;
  }
  clear() {
    this.items.clear();
  }
  getItem(key: string) {
    return this.items.get(key) ?? null;
  }
  key(index: number) {
    return [...this.items.keys()][index] ?? null;
  }
  removeItem(key: string) {
    this.items.delete(key);
  }
  setItem(key: string, value: string) {
    this.items.set(key, value);
  }
}

let storage: MemoryStorage;

function install(value: Storage | (() => never)): void {
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    get: typeof value === "function" ? value : () => value,
  });
}

beforeEach(() => {
  storage = new MemoryStorage();
  install(storage);
  clearToken();
});

afterEach(() => {
  Reflect.deleteProperty(globalThis, "isTauri");
});

describe("token storage", () => {
  test("keeps the token across reloads and removes it on sign-out", () => {
    setToken("abc");
    expect(storage.getItem("copilot.token")).toBe("abc");
    expect(getToken()).toBe("abc");
    clearToken();
    expect(storage.getItem("copilot.token")).toBeNull();
    expect(getToken()).toBeNull();
  });

  test("notifies subscribers until they unsubscribe", () => {
    let changes = 0;
    const unsubscribe = subscribeToken(() => changes++);
    setToken("abc");
    clearToken();
    clearToken();
    unsubscribe();
    setToken("def");
    expect(changes).toBe(2);
  });

  test("works in memory when storage is blocked", () => {
    install(() => {
      throw new DOMException("Blocked", "SecurityError");
    });
    setToken("abc");
    expect(getToken()).toBe("abc");
    clearToken();
    expect(getToken()).toBeNull();
  });

  test("follows sign-in and sign-out in another tab", () => {
    let changes = 0;
    const unsubscribe = subscribeToken(() => changes++);
    storage.setItem("copilot.token", "from-other-tab");
    followStorage({ key: "copilot.token" });
    expect(getToken()).toBe("from-other-tab");
    storage.removeItem("copilot.token");
    followStorage({ key: "copilot.token" });
    expect(getToken()).toBeNull();
    followStorage({ key: "unrelated" });
    unsubscribe();
    expect(changes).toBe(2);
  });
});

describe("server URL", () => {
  test("accepts http and https URLs and drops the trailing slash", () => {
    expect(parseServerUrl(" https://copilot.example/ ")).toBe("https://copilot.example");
    expect(parseServerUrl("http://192.168.1.20:3000")).toBe("http://192.168.1.20:3000");
    expect(parseServerUrl("https://example.com/copilot//")).toBe("https://example.com/copilot");
  });

  test("rejects other schemes, credentials, queries, and fragments", () => {
    for (const value of [
      "copilot.example",
      "ftp://copilot.example",
      "javascript:alert(1)",
      "https://user:pass@copilot.example",
      "https://copilot.example/?a=1",
      "https://copilot.example/#x",
    ]) {
      expect(parseServerUrl(value)).toBeUndefined();
    }
  });

  test("web builds call their own origin unless PUBLIC_API_URL is set", () => {
    expect(isNative()).toBe(false);
    expect(buildServerUrl()).toBe("");
    expect(serverUrl()).toBe("");
  });

  test("native builds use the stored URL, and changing it signs out", () => {
    Object.defineProperty(globalThis, "isTauri", { value: true, configurable: true });
    expect(isNative()).toBe(true);
    expect(serverUrl()).toBeUndefined();

    saveServerUrl("https://copilot.example/");
    expect(serverUrl()).toBe("https://copilot.example");
    setToken("abc");

    saveServerUrl("https://copilot.example");
    expect(getToken()).toBe("abc");

    saveServerUrl("https://other.example");
    expect(getToken()).toBeNull();

    setToken("def");
    forgetServerUrl();
    expect(serverUrl()).toBeUndefined();
    expect(getToken()).toBeNull();
  });

  test("only the native app on Android handles the back button", () => {
    const real = Object.getOwnPropertyDescriptor(globalThis, "navigator");
    const agent = (userAgent: string) =>
      Object.defineProperty(globalThis, "navigator", { configurable: true, value: { userAgent } });
    try {
      const phone = "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 Chrome/142.0";
      const desktop = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)";
      agent(phone);
      expect(isAndroid()).toBe(false);
      Object.defineProperty(globalThis, "isTauri", { value: true, configurable: true });
      expect(isAndroid()).toBe(true);
      agent(desktop);
      expect(isAndroid()).toBe(false);
    } finally {
      if (real) Object.defineProperty(globalThis, "navigator", real);
    }
  });

  test("refuses to store an invalid URL", () => {
    expect(() => saveServerUrl("not a url")).toThrow("Enter a full http:// or https:// address.");
  });
});
