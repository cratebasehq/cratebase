import { describe, expect, test } from "bun:test";

import { displayPath, nextSteps } from "../src/output.js";
import { packageManagerFromUserAgent } from "../src/post-steps.js";
import { devCommand, scriptCommandLine } from "../src/templates.js";

describe("nextSteps", () => {
  const base = { relDir: "my-app", pm: "bun", template: "vite-react", installed: true, cratebaseInstalled: true } as const;

  test("cd, dev command, URLs and demo login", () => {
    const out = nextSteps(base);
    expect(out).toContain("1. cd my-app");
    expect(out).toContain("2. bun run dev");
    expect(out).toContain("http://localhost:5173");
    expect(out).toContain("demo@example.com / password123");
  });

  test("adds an install step and the cratebase binary hint when needed", () => {
    const out = nextSteps({ ...base, installed: false, cratebaseInstalled: false, pm: "pnpm" });
    expect(out).toContain("install.sh");
    expect(out).toContain("pnpm install");
    expect(out).toContain("pnpm dev");
  });

  test("omits cd for the current directory and quotes paths with spaces", () => {
    expect(nextSteps({ ...base, relDir: "." })).toContain("1. bun run dev");
    expect(nextSteps({ ...base, relDir: "my app" })).toContain('cd "my app"');
  });
});

describe("displayPath", () => {
  test("relative inside cwd, absolute outside", () => {
    expect(displayPath("/work", "/work/app")).toBe("app");
    expect(displayPath("/work", "/work")).toBe(".");
    expect(displayPath("/work", "/tmp/app")).toBe("/tmp/app");
  });
});

describe("package manager helpers", () => {
  test("reads the user agent", () => {
    expect(packageManagerFromUserAgent("bun/1.3.0 npm/? node/v24")).toBe("bun");
    expect(packageManagerFromUserAgent("pnpm/9.0.0 npm/? node/v22")).toBe("pnpm");
    expect(packageManagerFromUserAgent("yarn/1.22.0")).toBe("yarn");
    expect(packageManagerFromUserAgent("npm/10.0.0 node/v22")).toBe("npm");
    expect(packageManagerFromUserAgent(undefined)).toBeUndefined();
    expect(packageManagerFromUserAgent("deno/2.0")).toBeUndefined();
  });

  test("script command lines", () => {
    expect(devCommand("npm")).toBe("npm run dev");
    expect(devCommand("yarn")).toBe("yarn dev");
    expect(scriptCommandLine("pnpm", "typegen")).toBe("pnpm typegen");
  });
});
