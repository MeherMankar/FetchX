const fs = require("fs");
const path = require("path");
const { minify } = require("terser");

const SRC = __dirname;
const read = f => fs.readFileSync(path.join(SRC, f), "utf8");

function stripEsm(code) {
  return code
      .replace(/^\s*import\s+[^\n]*?;?\s*$/gm, "")
      .replace(/^\s*export\s+(?=class|function|const|let|var)/gm, "");
}

const forge = read("forge.min.js");
const protobuf = read("protobuf.min.js");
const licenseProtocol = read("license_protocol.min.js");
const cmac = stripEsm(read("cmac.js"));
const session = stripEsm(read("session.js"));
const page_eme = stripEsm(read("page_eme.js"));
const page_web = stripEsm(read("page_web.js"));
const util = stripEsm(read("util.js"));

const header = `(function (__g) {
var __hadForge = ("forge" in __g), __prevForge = __g.forge;
var __hadProto = ("protobuf" in __g), __prevProto = __g.protobuf;
var module, exports, define;
var process = { versions: {} };
`;

const footer = `
if (__hadForge) { __g.forge = __prevForge; } else { try { delete __g.forge; } catch (e) { __g.forge = undefined; } }
if (__hadProto) { __g.protobuf = __prevProto; } else { try { delete __g.protobuf; } catch (e) { __g.protobuf = undefined; } }
})(typeof globalThis !== "undefined" ? globalThis
 : typeof self !== "undefined" ? self
 : typeof window !== "undefined" ? window : this);
`;

const bundle = [
  header,
  "/* ===== forge.min.js ===== */",
  forge,
  "\nvar forge = __g.forge;",
  "/* ===== protobuf.min.js ===== */",
  protobuf,
  "\nvar protobuf = __g.protobuf;",
  "/* ===== license_protocol.min.js ===== */",
  licenseProtocol,
  "/* ===== util.js ===== */",
  util,
  "/* ===== cmac.js ===== */",
  cmac,
  "/* ===== session.js ===== */",
  session,
  "/* ===== page_eme.js ===== */",
  page_eme,
  "/* ===== page_web.js ===== */",
  page_web,
  footer,
].join("\n");

(async () => {
  const result = await minify(bundle, {
    compress: { passes: 2 },
    mangle: true,
    format: { comments: false },
  });
  if (result.error) throw result.error;
  fs.writeFileSync(path.join(SRC, "bundle.min.js"), result.code);
  console.log("minified bytes:", result.code.length, "-> bundle.min.js");
})();