type Container =
  | { kind: "object"; value: Record<string, unknown>; key: string | undefined }
  | { kind: "array"; value: unknown[] };

const jsonNumbers = new WeakMap<object, number>();

/** Only number schemas may erase a JSON token's non-integer lexical form. */
export function unwrapJsonNumber(value: unknown): unknown {
  return typeof value === "function" ? jsonNumbers.get(value) ?? value : value;
}

function jsonNumber(token: string): unknown {
  const value = Number(token);
  if (/^(?:0|-?[1-9][0-9]*)$/.test(token)) return value;
  // Functions cannot pass JSON object, array or scalar schemas (unlike boxed
  // numbers or empty class instances). This inert marker has no numeric payload
  // or executable behavior; unwrap only looks it up, never calls it. Weak keys
  // release numbers with their parsed response, without a process-wide cache.
  const marker = Object.freeze(() => {});
  jsonNumbers.set(marker, value);
  return marker;
}

/** Validate syntax natively, then preserve numeric tokens and unique decoded keys. */
export function parseJson(text: string): unknown {
  try {
    JSON.parse(text);
    let value: unknown;
    const stack: Container[] = [];
    const number = /-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/y;
    const append = (child: unknown) => {
      const parent = stack.at(-1);
      if (!parent) value = child;
      else if (parent.kind === "array") parent.value.push(child);
      else {
        if (parent.key === undefined) throw new Error();
        // Define an own property even for __proto__, just like JSON.parse.
        Object.defineProperty(parent.value, parent.key, { value: child, enumerable: true, configurable: true, writable: true });
        parent.key = undefined;
      }
    };
    // Walk source order, not native reviver order (which sorts integer keys).
    // The explicit stack also avoids a recursive parser's nesting limit.
    for (let offset = 0; offset < text.length; offset++) {
      const character = text[offset];
      if (character === '"') {
        const start = offset++;
        while (offset < text.length && text[offset] !== '"') {
          offset += text[offset] === "\\" ? 2 : 1;
        }
        const parent = stack.at(-1);
        const decoded: unknown = JSON.parse(text.slice(start, offset + 1));
        if (typeof decoded !== "string") throw new Error();
        if (parent?.kind === "object" && parent.key === undefined) {
          if (Object.hasOwn(parent.value, decoded)) throw new Error();
          parent.key = decoded;
        } else append(decoded);
      } else if (character === "{") {
        const object: Record<string, unknown> = {};
        append(object);
        stack.push({ kind: "object", value: object, key: undefined });
      } else if (character === "[") {
        const array: unknown[] = [];
        append(array);
        stack.push({ kind: "array", value: array });
      } else if (character === "}" || character === "]") {
        stack.pop();
      } else if (character === "t" || character === "f" || character === "n") {
        append(character === "t" ? true : character === "f" ? false : null);
        offset += character === "f" ? 4 : 3;
      } else if (character !== "," && character !== ":" && character !== " " && character !== "\t" && character !== "\r" && character !== "\n") {
        number.lastIndex = offset;
        const match = number.exec(text);
        if (!match) throw new Error();
        append(jsonNumber(match[0]));
        offset = number.lastIndex - 1;
      }
    }
    return value;
  } catch {
    // Native SyntaxError messages may include the raw response. Never retain it.
    throw new SyntaxError("Invalid JSON response");
  }
}
