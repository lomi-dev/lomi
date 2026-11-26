import { Node } from "@tiptap/core";

/** Atomic inline pill; `attr` holds the value serialized by composer-doc. */
const pill = (name: string, attr: string, label: (value: string) => string) =>
  Node.create({
    name,
    group: "inline",
    inline: true,
    atom: true,
    addAttributes: () => ({ [attr]: { default: "" } }),
    parseHTML: () => [],
    renderHTML: ({ node }) => [
      "span",
      {
        class: "composer-pill",
        title: String(node.attrs[attr]),
      },
      label(String(node.attrs[attr])),
    ],
  });

export const FileMention = pill("fileMention", "path", (path) =>
  path.slice(path.lastIndexOf("/") + 1),
);
export const SlashCommand = pill("slashCommand", "name", (name) => `/${name}`);
