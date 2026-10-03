type Container =
  | { kind: "object"; keys: Set<string>; expectingKey: boolean }
  | { kind: "array" };

/** JSON syntax stays with the native parser; decoded object keys must be unique. */
export function parseJson(text: string): unknown {
  try {
    const value: unknown = JSON.parse(text);
    const stack: Container[] = [];
    for (let offset = 0; offset < text.length; offset++) {
      const character = text[offset];
      if (character === '"') {
        const start = offset++;
        while (offset < text.length && text[offset] !== '"') {
          offset += text[offset] === "\\" ? 2 : 1;
        }
        const parent = stack.at(-1);
        if (parent?.kind === "object" && parent.expectingKey) {
          const key: unknown = JSON.parse(text.slice(start, offset + 1));
          if (typeof key !== "string" || parent.keys.has(key)) throw new Error();
          parent.keys.add(key);
          parent.expectingKey = false;
        }
      } else if (character === "{") {
        stack.push({ kind: "object", keys: new Set(), expectingKey: true });
      } else if (character === "[") {
        stack.push({ kind: "array" });
      } else if (character === "}" || character === "]") {
        stack.pop();
      } else if (character === ",") {
        const parent = stack.at(-1);
        if (parent?.kind === "object") parent.expectingKey = true;
      }
    }
    return value;
  } catch {
    // Native SyntaxError messages may include the raw response. Never retain it.
    throw new SyntaxError("Invalid JSON response");
  }
}
