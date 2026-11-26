// The installable web app (manifest and icons) and the dev server's API proxy.

import { existsSync, readFileSync } from "node:fs";
import { describe, expect, test } from "@rstest/core";

/** `rsbuild.config.ts` as text (tests may not import it with a relative path). */
const config = readFileSync("rsbuild.config.ts", "utf8");

type Icon = { src: string; sizes: string; type: string; purpose?: string };

const manifest = JSON.parse(readFileSync("public/manifest.webmanifest", "utf8")) as {
  name: string;
  short_name: string;
  start_url: string;
  scope: string;
  display: string;
  icons: Icon[];
};

/** Width and height from a PNG's IHDR chunk. */
function pngSize(file: string): string {
  const bytes = readFileSync(file);
  expect(bytes.subarray(1, 4).toString("ascii")).toBe("PNG");
  return `${bytes.readUInt32BE(16)}x${bytes.readUInt32BE(20)}`;
}

describe("web app manifest", () => {
  test("describes a standalone app that starts at the root", () => {
    expect(manifest.name).toBe("Copilot");
    expect(manifest.short_name).toBe("Copilot");
    expect(manifest.start_url).toBe("/");
    expect(manifest.scope).toBe("/");
    expect(manifest.display).toBe("standalone");
  });

  test("lists 192 and 512 pixel icons and a maskable one, all present at their sizes", () => {
    const sizes = manifest.icons.map(
      (icon) => `${icon.sizes}${icon.purpose ? ` ${icon.purpose}` : ""}`
    );
    expect(sizes).toEqual(expect.arrayContaining(["192x192", "512x512", "512x512 maskable"]));
    for (const icon of manifest.icons) {
      const file = `public${icon.src}`;
      expect([file, existsSync(file)]).toEqual([file, true]);
      if (icon.type === "image/png") expect(pngSize(file)).toBe(icon.sizes);
    }
  });

  test("the page links the manifest and its icons", () => {
    for (const href of [
      "/manifest.webmanifest",
      "/icons/icon.svg",
      "/icons/apple-touch-icon.png",
    ]) {
      expect(config).toContain(`"${href}"`);
      expect(existsSync(`public${href}`)).toBe(true);
    }
    expect(pngSize("public/icons/apple-touch-icon.png")).toBe("180x180");
  });
});

describe("dev server", () => {
  test("proxies the API and the health checks to the API on port 3000", () => {
    expect(config).toContain('|| "http://localhost:3000"');
    expect(config).toContain('"/api": apiTarget');
    expect(config).toContain('"/health": apiTarget');
    expect(config).toContain("port: 4000");
  });

  test("keeps the composer above the on-screen keyboard and offers both color schemes", () => {
    expect(config).toContain("interactive-widget=resizes-content");
    expect(config).toContain('"color-scheme": "light dark"');
  });
});
