import assert from "node:assert/strict";
import test from "node:test";
import {
  layoutPanes,
  newTab,
  newThreadTab,
  renameTabItem,
  type Session,
  type Tab,
} from "./model.ts";

const withTabs = (tabs: Tab[]) =>
  ({ projects: [{ workspaces: [{ tabs }] }] }) as unknown as Session;
const tabsOf = (session: Session) => session.projects[0].workspaces[0].tabs;

test("renameTabItem names a tab, a lone pane with its tab, or one split pane", () => {
  const chat = newThreadTab();
  const [named] = tabsOf(
    renameTabItem(withTabs([chat]), chat.id, chat.id, "A"),
  );
  assert.equal(named.customTitle, "A");

  const lone = newTab("/w", "sh");
  const pane = layoutPanes(lone.layout)[0];
  const [loneNamed] = tabsOf(
    renameTabItem(withTabs([lone]), lone.id, pane.id, "B"),
  );
  assert.equal(loneNamed.customTitle, "B");
  assert.equal(
    loneNamed.type === "terminal" &&
      layoutPanes(loneNamed.layout)[0].customTitle,
    "B",
  );

  const split = newTab("/w", "sh", "Terminal", 2);
  const [first, second] = layoutPanes(split.layout);
  const [splitNamed] = tabsOf(
    renameTabItem(withTabs([split]), split.id, second.id, "C"),
  );
  assert.equal(splitNamed.customTitle, undefined);
  const panes =
    splitNamed.type === "terminal" ? layoutPanes(splitNamed.layout) : [];
  assert.deepEqual(
    panes.map((p) => [p.id, p.customTitle]),
    [
      [first.id, undefined],
      [second.id, "C"],
    ],
  );
});
