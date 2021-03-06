import fs from "fs";
import fetch from "node-fetch";
import { parse } from "node-html-parser";
import {
  Parser,
  EmitFormat,
  Production,
  RightHandSide,
  RightHandSideList,
  SymbolSpan,
  SymbolSet,
  SyntaxKind,
  OneOfList,
} from "grammarkdown";
import { decode } from "html-entities";

try {
  fs.mkdirSync("workdir");
} catch {}

// 1. fetch `spec.html` if it does not exist

if (!fs.existsSync("workdir/spec.html"))
  await fetch("https://github.com/tc39/ecma262/blob/es2021/spec.html?raw=true")
    .then((resp) => resp.text())
    .then((text) => {
      fs.writeFileSync("workdir/spec.html", text);
    });

// 2. grab all grammar notation sections
const spec = fs.readFileSync("workdir/spec.html", { encoding: "utf-8" });

let didHitLanguageExprsSection = false;
const grammarSections = parse(spec)
  .getElementsByTagName("emu-grammar")
  .filter((element) => element.attributes["type"] === "definition")
  .filter((element) => !("example" in element.attributes))
  .filter((element) => {
    didHitLanguageExprsSection =
      didHitLanguageExprsSection ||
      element.parentNode.parentNode.id ===
        "sec-ecmascript-language-expressions";
    return didHitLanguageExprsSection;
  })
  .filter((element) => {
    const isRegexSection =
      element.parentNode.parentNode.id ===
        "sec-regexp-regular-expression-objects" ||
      element.parentNode.id === "sec-regular-expressions-patterns";
    return !isRegexSection;
  })
  .filter((element) => {
    const isForWeb =
      element.parentNode.parentNode.parentNode.id ===
      "sec-additional-ecmascript-features-for-web-browsers";
    return !isForWeb;
  })
  .filter((element) => {
    const isNativeFn =
      element.parentNode.id === "sec-function.prototype.tostring";
    return !isNativeFn;
  })
  .filter((element) => {
    const isUriHandling =
      element.parentNode.parentNode.id === "sec-uri-handling-functions";
    return !isUriHandling;
  })
  .map((element) => element.innerText);

// 3. parse all language grammar sections
const parser = new Parser();

/** @type {[string, Production][]} */
const productions = grammarSections
  .map((grammar) => [grammar, parser.parseSourceFile("...", grammar)])
  .flatMap(([grammar, file]) =>
    file.elements.map((element) => [[grammar, file], element])
  )
  .filter(([_, sourceElement]) => sourceElement instanceof Production);

const ast = productions
  .filter(([_, { body }]) => body instanceof RightHandSideList)
  .map(([[grammar, sourceFile], { name, body: prodBody }]) => {
    /** @type {RightHandSide[]} */
    const body = (prodBody && prodBody.elements) ?? [];

    return {
      name: name.text,
      body: body.map((rhs) => {
        try {
          return {
            source: rhs.getText(sourceFile),
            sequence: mapRhs(rhs.head),
          };
        } catch (err) {
          console.log(
            "error while parsing",
            grammar,
            { name: name.text },
            rhs.head
          );
          throw err;
        }
      }),
    };
  });

const oneOfAst = productions
  .filter(([grammar, { body }]) => body instanceof OneOfList)
  .map(([grammar, { name, body: body2 }]) => {
    /** @type {OneOfList} */
    const body = body2;
    const terminals = body.terminals ?? [];

    return {
      name: name.text,
      terminals: terminals.map((terminal) => decode(terminal.text)),
    };
  });

/** @param {SymbolSpan[] | SymbolSpan | undefined} symbolSpan */
function mapRhs(symbolSpan, symbols = []) {
  if (!symbolSpan) return symbols;
  if (Array.isArray(symbolSpan)) return symbolSpan.map((elem) => mapRhs(elem));

  handleSymbol(symbolSpan.symbol, symbols);

  return mapRhs(symbolSpan.next, symbols);
}

/** @param {import("grammarkdown").LexicalSymbol | import("grammarkdown").LexicalSymbol[]} symbol */
function handleSymbol(symbol, array) {
  if (Array.isArray(symbol)) {
    symbol.forEach((elem) => handleSymbol(elem, array));
    return;
  }

  if ("literal" in symbol) array.push({ literal: symbol.literal.text });
  else if ("symbol" in symbol) array.push({ symbol: symbol.symbol.text });
  else if ("name" in symbol) {
    const element = {
      name: symbol.name.text,
    };

    if (symbol.questionToken !== undefined) {
      element.optional = true;
    }

    array.push({ name: element });
  } else if ("lookahead" in symbol) {
    // we don't care about lookahead
    // const lookahead = symbol.lookahead;
    // if (lookahead instanceof SymbolSet) array.push(mapRhs(lookahead.elements));
    // else {
    //   handleSymbol(lookahead, array);
    // }
  } else if ("symbols" in symbol && symbol.symbols) {
    let oneOfItems = [];
    let emitSymbol = true;

    for (const subSymbol of symbol.symbols) {
      if ("literal" in subSymbol) {
        oneOfItems.push(subSymbol.literal.text);
      } else if (
        "name" in subSymbol &&
        subSymbol.name.text === "LineTerminator"
      ) {
        emitSymbol = false;
      } else {
        console.warn("sub", subSymbol);
        throw new Error("unknown subsymbol");
      }
    }

    if (emitSymbol) array.push({ oneOf: oneOfItems });
  } else if ("butKeyword" in symbol && "notKeyword" in symbol) {
    // ignore the "but not" part
    handleSymbol(symbol.left, array);
  } else if ("emptyKeyword" in symbol) {
    // push nothing into `array`, as it should be empty
  } else {
    console.warn("unhandled", symbol);
    throw new Error("unhandled symbol");
  }
}

const totalAst = { ast, oneOfAst };

// 4. save AST to JSON to then give to rust code for code generation
const grammarNotation = "workdir/grammar-notation.json";
fs.writeFileSync(grammarNotation, JSON.stringify(totalAst));

// 5. update `parse_nodes`
const parseNodes = "../compiler/src/frontend/js/ast/parse_nodes.json";
fs.rmSync(parseNodes);
fs.copyFileSync(grammarNotation, parseNodes);
