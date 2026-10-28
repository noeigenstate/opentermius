import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/icons.tsx", import.meta.url), "utf8");
const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const jsx = moduleUrl("export function jsx(type, props) { return { type, props }; }");
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
}).outputText.replace('"react/jsx-runtime"', JSON.stringify(jsx));
const { Icon } = await import(moduleUrl(compiled));

test("OS glyphs are single filled paths while existing icons keep their strokes", () => {
  for (const name of ["osUbuntu", "osDebian", "osCentos", "osRhel", "osFedora", "osRocky", "osAlmalinux", "osAlpine", "osArch", "osOpensuse", "osAmazon", "osKali", "osRaspbian", "osLinux", "osFreebsd", "osMacos", "osWindows"]) {
    const icon = Icon({ name, size: 22 });
    assert.equal(icon.props.viewBox, "0 0 24 24");
    assert.equal(icon.props.fill, "currentColor");
    assert.equal(icon.props.stroke, "none");
    assert.equal(icon.props.children.type, "path");
    assert(icon.props.children.props.d.length > 0);
  }
  for (const name of ["hosts", "more", "star", "list", "info", "chevronLeft", "chevronRight"]) {
    const icon = Icon({ name });
    assert.equal(icon.props.fill, "none");
    assert.equal(icon.props.stroke, "currentColor");
    assert.equal(icon.props.strokeWidth, name === "more" ? 3.2 : 1.8);
  }
});
