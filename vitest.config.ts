import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

// 测试：npm test。和打包时一样认 @shared
export default defineConfig({
  resolve: { alias: { "@shared": fileURLToPath(new URL("./shared", import.meta.url)) } },
  test: { include: ["apps/**/*.test.ts", "shared/**/*.test.ts"] },
});
