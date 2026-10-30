import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";

// Fails when a locked npm package or crate version is younger than the
// cooldown, so a compromised release has time to be found and yanked first.
const COOLDOWN_DAYS = 14;
const repo = fileURLToPath(new URL("..", import.meta.url));
// Security fixes may skip the cooldown. Each entry names its advisory. The
// check fails once an entry is old enough, so remove it then.
const EXEMPT = {
  "crates.io:rustls-webpki@0.103.16": "RUSTSEC-2026-0382",
  "crates.io:webpki-roots@1.0.10": "RUSTSEC-2026-0381",
};
const cacheDirectory = `${homedir()}/.cache/truffle`;
const cachePath = `${cacheDirectory}/release-times.json`;

const npmLocked = (path) =>
  Object.entries(JSON.parse(readFileSync(`${repo}${path}`, "utf8")).packages)
    .filter(([, entry]) =>
      entry.resolved?.startsWith("https://registry.npmjs.org/"),
    )
    .map(([key, entry]) => ({
      registry: "npm",
      name: key.slice(
        key.lastIndexOf("node_modules/") + "node_modules/".length,
      ),
      version: entry.version,
    }));

const cargoLocked = () =>
  readFileSync(`${repo}desktop/Cargo.lock`, "utf8")
    .split("[[package]]")
    .map((block) => ({
      name: /^name = "([^"]+)"/m.exec(block)?.[1],
      version: /^version = "([^"]+)"/m.exec(block)?.[1],
      source: /^source = "([^"]+)"/m.exec(block)?.[1],
    }))
    .filter(
      ({ source }) =>
        source === "registry+https://github.com/rust-lang/crates.io-index",
    )
    .map(({ name, version }) => ({ registry: "crates.io", name, version }));

function indexPath(name) {
  const lower = name.toLowerCase();
  switch (lower.length) {
    case 1:
      return `1/${lower}`;
    case 2:
      return `2/${lower}`;
    case 3:
      return `3/${lower[0]}/${lower}`;
    default:
      return `${lower.slice(0, 2)}/${lower.slice(2, 4)}/${lower}`;
  }
}

async function fetchTimes(registry, name) {
  if (registry === "npm") {
    const response = await fetch(
      `https://registry.npmjs.org/${name.replace("/", "%2F")}`,
    );
    if (!response.ok) {
      throw new Error(`npm ${name}: HTTP ${response.status}`);
    }
    return (await response.json()).time;
  }
  const response = await fetch(`https://index.crates.io/${indexPath(name)}`);
  if (!response.ok) {
    throw new Error(`crates.io ${name}: HTTP ${response.status}`);
  }
  return Object.fromEntries(
    (await response.text())
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line))
      .map((entry) => [entry.vers, entry.pubtime]),
  );
}

let cache = {};
try {
  cache = JSON.parse(readFileSync(cachePath, "utf8"));
} catch {
  // A missing or unreadable cache only means more requests.
}

const locked = [
  ...npmLocked("package-lock.json"),
  ...npmLocked("desktop/package-lock.json"),
  ...cargoLocked(),
];
const key = ({ registry, name, version }) => `${registry}:${name}@${version}`;
const missing = [
  ...new Map(
    locked
      .filter((item) => !(key(item) in cache))
      .map((item) => [`${item.registry}:${item.name}`, item]),
  ).values(),
];
const failures = [];
for (let start = 0; start < missing.length; start += 16) {
  const batch = missing.slice(start, start + 16);
  const results = await Promise.allSettled(
    batch.map(({ registry, name }) => fetchTimes(registry, name)),
  );
  for (const [index, result] of results.entries()) {
    const { registry, name } = batch[index];
    if (result.status === "rejected") {
      failures.push(result.reason.message);
      continue;
    }
    for (const [version, time] of Object.entries(result.value ?? {})) {
      if (typeof time === "string") {
        cache[`${registry}:${name}@${version}`] = time;
      }
    }
  }
}
mkdirSync(cacheDirectory, { recursive: true });
writeFileSync(cachePath, JSON.stringify(cache));

const cutoff = Date.now() - COOLDOWN_DAYS * 24 * 60 * 60 * 1000;
const lockedKeys = new Set(locked.map(key));
for (const item of locked) {
  const time = cache[key(item)];
  if (time === undefined) {
    failures.push(`${key(item)}: no publish time found`);
  } else if (Date.parse(time) > cutoff && !(key(item) in EXEMPT)) {
    failures.push(`${key(item)}: published ${time}`);
  }
}
for (const exempt of Object.keys(EXEMPT)) {
  if (!lockedKeys.has(exempt) || Date.parse(cache[exempt]) <= cutoff) {
    failures.push(`${exempt}: remove its exemption; it is not needed`);
  }
}
if (failures.length > 0) {
  console.error(
    `Locked versions must be at least ${COOLDOWN_DAYS} days old:\n  ${[...new Set(failures)].join("\n  ")}`,
  );
  process.exit(1);
}
console.log(
  `${locked.length} locked versions pass the ${COOLDOWN_DAYS}-day cooldown.`,
);
