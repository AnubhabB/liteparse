// `batch-parse` input selection / output naming and `--version`.
//
// These drive the built CLI (dist/cli.js) like cli-text.test.mjs does.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageDir, "..", "..");
const cliPath = join(packageDir, "dist", "cli.js");
const samplePdf = join(repoRoot, "integration_tests_data", "sample.pdf");
const sampleImage = join(repoRoot, "integration_tests_data", "receipt.png");

function runCli(args) {
  // The timeout keeps a regression from hanging the suite.
  return spawnSync(process.execPath, [cliPath, ...args], { encoding: "utf8", timeout: 60_000 });
}

function batchParse(inputDir, outputDir, ...extra) {
  return runCli(["batch-parse", inputDir, outputDir, "--no-ocr", "--max-pages", "1", ...extra]);
}

function withTempDirs(fn) {
  const dir = mkdtempSync(join(tmpdir(), "liteparse-cli-batch-"));
  const inputDir = join(dir, "input");
  const outputDir = join(dir, "output");
  mkdirSync(inputDir);
  try {
    return fn(inputDir, outputDir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("--version reports the version in package.json", () => {
  const { version } = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"));
  const result = runCli(["--version"]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.trim(), version);
});

test("batch-parse refuses inputs that map to the same output file", () => {
  withTempDirs((inputDir, outputDir) => {
    copyFileSync(samplePdf, join(inputDir, "report.pdf"));
    copyFileSync(sampleImage, join(inputDir, "report.png"));

    const result = batchParse(inputDir, outputDir);

    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /output path collision/);
    assert.match(result.stderr, /report\.pdf/);
    assert.match(result.stderr, /report\.png/);
    // Nothing was parsed or written.
    assert.deepEqual(readdirSync(outputDir), []);
  });
});

test("batch-parse --extension avoids a stem collision", () => {
  withTempDirs((inputDir, outputDir) => {
    copyFileSync(samplePdf, join(inputDir, "report.pdf"));
    copyFileSync(sampleImage, join(inputDir, "report.png"));

    const result = batchParse(inputDir, outputDir, "--extension", "pdf", "--quiet");

    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(readdirSync(outputDir), ["report.txt"]);
  });
});

test("batch-parse allows the same stem in different directories", () => {
  withTempDirs((inputDir, outputDir) => {
    for (const sub of ["a", "b"]) {
      mkdirSync(join(inputDir, sub));
      copyFileSync(samplePdf, join(inputDir, sub, "report.pdf"));
    }

    const result = batchParse(inputDir, outputDir, "--recursive", "--quiet");

    assert.equal(result.status, 0, result.stderr);
    assert.equal(existsSync(join(outputDir, "a", "report.txt")), true);
    assert.equal(existsSync(join(outputDir, "b", "report.txt")), true);
  });
});

test(
  "batch-parse detects collisions that differ only by case on case-insensitive file systems",
  { skip: process.platform !== "win32" && process.platform !== "darwin" },
  () => {
    withTempDirs((inputDir, outputDir) => {
      copyFileSync(samplePdf, join(inputDir, "Report.pdf"));
      copyFileSync(sampleImage, join(inputDir, "report.png"));

      const result = batchParse(inputDir, outputDir);

      assert.equal(result.status, 1, result.stderr);
      assert.match(result.stderr, /output path collision/);
    });
  }
);

test("batch-parse skips plain-text files the parser cannot handle", () => {
  withTempDirs((inputDir, outputDir) => {
    copyFileSync(samplePdf, join(inputDir, "doc.pdf"));
    for (const name of ["notes.txt", "README.md", "CHANGES.markdown", "run.log"]) {
      writeFileSync(join(inputDir, name), "plain text\n");
    }

    const result = batchParse(inputDir, outputDir);

    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stderr, /found 1 files to process/);
    assert.match(result.stderr, /batch complete: 1 succeeded, 0 failed/);
    assert.deepEqual(readdirSync(outputDir), ["doc.txt"]);
  });
});
