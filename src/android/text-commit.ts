/** Assemble adjacent UTF-16 text chunks while retaining at most one high half. */
export class TextCommitAssembler {
  private trailingHigh = "";

  append(text: string): string {
    const source = this.trailingHigh + text;
    this.trailingHigh = "";
    let complete = "";
    for (let index = 0; index < source.length; index++) {
      const unit = source.charCodeAt(index);
      if (unit >= 0xd800 && unit <= 0xdbff) {
        const next = source.charCodeAt(index + 1);
        if (next >= 0xdc00 && next <= 0xdfff) {
          complete += source.slice(index, index + 2);
          index++;
        } else if (index + 1 === source.length) {
          this.trailingHigh = source[index];
        }
      } else if (unit < 0xdc00 || unit > 0xdfff) {
        complete += source[index];
      }
    }
    return complete;
  }

  reset() {
    this.trailingHigh = "";
  }
}

/** Composition snapshots and paste are complete strings, never continuation chunks. */
export function wellFormedInputText(text: string): string {
  return new TextCommitAssembler().append(text);
}
