// The screenshots of the release notes come from the project website, and only images may:
// `img-src` is the one directive that names it. Everything else of the policy stays as it was.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const conf = JSON.parse(readFileSync(resolve(__dirname, "../../../../src-tauri/tauri.conf.json"), "utf8")) as {
  app: { security: { csp: string; devCsp: string } };
};

function directives(policy: string): Map<string, string> {
  return new Map(
    policy.split(";").map((part) => {
      const [name = "", ...values] = part.trim().split(/\s+/);
      return [name, values.join(" ")];
    }),
  );
}

describe("content security policy", () => {
  for (const key of ["csp", "devCsp"] as const) {
    it(`${key} lets images come from the app, data, blob and the website only`, () => {
      const policy = directives(conf.app.security[key]);
      expect(policy.get("img-src")).toBe("'self' data: blob: https://portolano.cloud");
      expect(policy.get("default-src")).toBe("'self'");
      expect(policy.get("object-src")).toBe("'none'");
      expect(policy.get("base-uri")).toBe("'none'");
      expect(policy.get("form-action")).toBe("'none'");
      expect(policy.get("style-src")).toBe("'self' 'unsafe-inline'");
      expect(policy.has("script-src")).toBe(false);
      expect(policy.get("connect-src")).not.toContain("portolano.cloud");
    });
  }
});
