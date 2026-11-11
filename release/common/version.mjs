// The release version is the single line of the VERSION file at the repository root. Run directly,
// this script prints it.
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

export function workspaceVersion(directory = root) {
  const version = readFileSync(join(directory, "VERSION"), "utf8").trim();
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`VERSION is not a semantic version: ${version}`);
  }
  return version;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { console.log(workspaceVersion()); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
