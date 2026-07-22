import assert from "node:assert/strict";
import test from "node:test";

import {
  maturityForTitle,
  normalizeWinetricksOutput,
  parseBuiltinDllOverrides,
} from "./catalog-metadata.mjs";

test("normalizes local home paths without changing unrelated text", () => {
  const input = "Remove links to /home/alice and keep /home/bob unchanged";
  assert.equal(
    normalizeWinetricksOutput(input, "/home/alice"),
    "Remove links to $HOME and keep /home/bob unchanged",
  );
});

test("does not treat a broken-font diagnostic as a broken verb", () => {
  assert.equal(maturityForTitle("Check for broken fonts"), "metadata_only");
  assert.equal(
    maturityForTitle("MS Windows Media Encoder 9 (broken in Wine)"),
    "broken_upstream",
  );
});

test("extracts the continued builtin DLL list from the audited function", () => {
  const source = `
w_override_all_dlls()
{
    w_override_dlls builtin \\
        d3d11 d3d12 \\
        xinput1_3 \\

        # trailing continuation is intentional
}
`;
  assert.deepEqual(parseBuiltinDllOverrides(source), ["d3d11", "d3d12", "xinput1_3"]);
});

test("rejects a malformed builtin DLL list", () => {
  assert.throws(
    () => parseBuiltinDllOverrides("w_override_all_dlls()\n{\n  w_override_dlls builtin \\\n  d3d11\n}"),
    /line continuation/,
  );
});
