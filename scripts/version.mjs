import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const path = (name) => new URL("../" + name, import.meta.url);
const read = (name) => readFileSync(path(name), "utf8");
const write = (name, contents) => writeFileSync(path(name), contents);
const stableVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const cargoVersion = /(\[workspace\.package\]\r?\nversion = ")([^"]+)(")/;
const workspace = read("Cargo.toml");
const current = cargoVersion.exec(workspace)?.[2];
if (!current || !stableVersion.test(current)) {
  throw new Error("Cargo.toml must declare a MAJOR.MINOR.PATCH workspace version.");
}

const packages = ["web", "desktop"].map((folder) => {
  const manifestPath = folder + "/package.json";
  const lockPath = folder + "/package-lock.json";
  const manifest = JSON.parse(read(manifestPath));
  const lock = JSON.parse(read(lockPath));
  if (manifest.version !== current || lock.version !== current || lock.packages?.[""]?.version !== current) {
    throw new Error(folder + " manifest and lockfile must match the Rust workspace version " + current + ".");
  }
  return { manifestPath, lockPath, manifest, lock };
});

const [command, extra] = process.argv.slice(2);
if (!command || extra || command === "--help") {
  console.log("Usage: node scripts/version.mjs --check | patch | minor | major | MAJOR.MINOR.PATCH");
  process.exit(command === "--help" ? 0 : 1);
}
if (command === "--check") {
  console.log("Workspace version " + current);
  process.exit(0);
}

const parts = current.split(".").map(BigInt);
const bump = ["major", "minor", "patch"].indexOf(command);
let next = command;
if (bump >= 0) {
  next = parts.map((part, index) => index < bump ? part : index === bump ? part + 1n : 0n).join(".");
}
if (!stableVersion.test(next)) throw new Error("Use a MAJOR.MINOR.PATCH version or a named bump.");
const nextParts = next.split(".").map(BigInt);
const firstChange = nextParts.findIndex((part, index) => part !== parts[index]);
if (firstChange < 0 || nextParts[firstChange] < parts[firstChange]) {
  throw new Error("The next version must be greater than " + current + ".");
}

const files = ["Cargo.toml", "Cargo.lock", ...packages.flatMap((pkg) => [pkg.manifestPath, pkg.lockPath])];
const original = new Map(files.map((name) => [name, read(name)]));
try {
  write("Cargo.toml", workspace.replace(cargoVersion, (_, before, _version, after) => before + next + after));
  for (const { manifestPath, lockPath, manifest, lock } of packages) {
    manifest.version = next;
    lock.version = next;
    lock.packages[""].version = next;
    write(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
    write(lockPath, JSON.stringify(lock, null, 2) + "\n");
  }
  const cargo = spawnSync("cargo", ["update", "--workspace"], { cwd: root, stdio: "inherit" });
  if (cargo.error) throw cargo.error;
  if (cargo.status !== 0) throw new Error("Cargo could not update the workspace lockfile.");
} catch (error) {
  for (const [name, contents] of original) write(name, contents);
  throw error;
}
console.log("Updated workspace version from " + current + " to " + next + ".");
