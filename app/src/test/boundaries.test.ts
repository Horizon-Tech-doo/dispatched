// @vitest-environment node
import { ESLint } from "eslint";
import { describe, expect, it } from "vitest";

const eslint = new ESLint({ cwd: process.cwd() });

async function restricted(filePath: string, code: string): Promise<string[]> {
  const [result] = await eslint.lintText(code, { filePath });
  return result.messages.filter((m) => m.ruleId === "no-restricted-imports").map((m) => m.message);
}

describe("import boundaries", () => {
  it("ui cannot import app code", async () => {
    expect(await restricted("src/ui/components/x.tsx", `import { a } from "../../engine/commands";\nexport const b = a;`)).toHaveLength(1);
    expect(await restricted("src/ui/x.ts", `import { a } from "../shell/Shell";\nexport const b = a;`)).toHaveLength(1);
    expect(await restricted("src/ui/x.ts", `import { a } from "@/engine/commands";\nexport const b = a;`)).toHaveLength(1);
  });

  it("ui can import itself and packages", async () => {
    expect(await restricted("src/ui/components/x.tsx", `import { cn } from "../lib/cn";\nimport { clsx } from "clsx";\nexport const b = [cn, clsx];`)).toHaveLength(0);
  });

  it("engine cannot import ui or shell", async () => {
    expect(await restricted("src/engine/queries.ts", `import { a } from "../ui";\nexport const b = a;`)).toHaveLength(1);
    expect(await restricted("src/engine/queries.ts", `import { a } from "@/shell/state";\nexport const b = a;`)).toHaveLength(1);
  });

  it("only engine/ipc.ts imports Tauri", async () => {
    expect(await restricted("src/shell/x.ts", `import { invoke } from "@tauri-apps/api/core";\nexport const b = invoke;`)).toHaveLength(1);
    expect(await restricted("src/engine/ipc.ts", `import { invoke } from "@tauri-apps/api/core";\nexport const b = invoke;`)).toHaveLength(0);
  });

  it("shell cannot import the dev catalogue", async () => {
    expect(await restricted("src/shell/x.ts", `import { Catalogue } from "../dev/Catalogue";\nexport const b = Catalogue;`)).toHaveLength(1);
  });
});
