import { describe, expect, test } from "bun:test";
import { ArgParseError, parseArgs } from "../src/args.js";

describe("parseArgs", () => {
  test("defaults", () => {
    const opts = parseArgs([]);
    expect(opts.projectDir).toBeUndefined();
    expect(opts.template).toBeUndefined();
    expect(opts.pm).toBeUndefined();
    expect(opts.yes).toBe(false);
    expect(opts.install).toBe(true);
    expect(opts.git).toBe(true);
    expect(opts.help).toBe(false);
  });

  test("positional project directory", () => {
    const opts = parseArgs(["my-app"]);
    expect(opts.projectDir).toBe("my-app");
  });

  test("rejects more than one positional", () => {
    expect(() => parseArgs(["my-app", "other"])).toThrow(ArgParseError);
  });

  test("--template long and short form", () => {
    expect(parseArgs(["--template", "nextjs"]).template).toBe("nextjs");
    expect(parseArgs(["-t", "vite-react"]).template).toBe("vite-react");
    expect(parseArgs(["--template=expo"]).template).toBe("expo");
  });

  test("rejects invalid --template", () => {
    expect(() => parseArgs(["--template", "sveltekit"])).toThrow(ArgParseError);
    expect(() => parseArgs(["--template"])).toThrow(ArgParseError);
  });

  test("--pm long form and =value form", () => {
    expect(parseArgs(["--pm", "pnpm"]).pm).toBe("pnpm");
    expect(parseArgs(["--pm=yarn"]).pm).toBe("yarn");
  });

  test("rejects invalid --pm", () => {
    expect(() => parseArgs(["--pm", "cargo"])).toThrow(ArgParseError);
  });

  test("--yes / -y", () => {
    expect(parseArgs(["--yes"]).yes).toBe(true);
    expect(parseArgs(["-y"]).yes).toBe(true);
  });

  test("--no-install and --no-git", () => {
    const opts = parseArgs(["--no-install", "--no-git"]);
    expect(opts.install).toBe(false);
    expect(opts.git).toBe(false);
  });

  test("--install and --git re-enable after a --no-*", () => {
    const opts = parseArgs(["--no-install", "--install", "--no-git", "--git"]);
    expect(opts.install).toBe(true);
    expect(opts.git).toBe(true);
  });

  test("--help / -h and --version / -v", () => {
    expect(parseArgs(["--help"]).help).toBe(true);
    expect(parseArgs(["-h"]).help).toBe(true);
    expect(parseArgs(["--version"]).version).toBe(true);
    expect(parseArgs(["-v"]).version).toBe(true);
  });

  test("rejects unknown flags", () => {
    expect(() => parseArgs(["--bogus"])).toThrow(ArgParseError);
  });

  test("full non-interactive invocation", () => {
    const opts = parseArgs(["my-app", "--template", "vite-react", "--pm", "bun", "--yes", "--no-install"]);
    expect(opts).toEqual({
      projectDir: "my-app",
      template: "vite-react",
      pm: "bun",
      yes: true,
      install: false,
      git: true,
      help: false,
      version: false,
    });
  });
});
