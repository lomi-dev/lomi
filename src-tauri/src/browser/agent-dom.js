// Executed only in the private WKContentWorld, with a native NSDictionary argument.
// No page-world callback, Tauri invoke, arbitrary expression, or selector input.
const q = JSON.parse(payload);
let effectMayHaveStarted = false;
const fail = (error) =>
  JSON.stringify({ error, noEffect: !effectMayHaveStarted });
const expired = () => Date.now() > q.deadlineEpochMs;
if (expired()) return fail("DEADLINE_EXCEEDED");
if (location.origin !== q.origin || location.href !== q.url)
  return fail("STALE_SNAPSHOT");
if (q.action === "geometry")
  return JSON.stringify({
    url: location.href,
    width: innerWidth,
    height: innerHeight,
    deviceScaleFactor: devicePixelRatio,
    scrollX,
    scrollY,
  });
if (q.action === "logs") {
  const state = globalThis.__lomiAgentLogsV1;
  if (!state) return fail("UNSUPPORTED_CAPABILITY");
  if (q.after >= state.next) return fail("CURSOR_EXPIRED");
  const entries = state.entries
    .filter((e) => e.sequence > q.after)
    .slice(0, q.limit);
  return JSON.stringify({
    captureStartedAtMillis: state.started,
    entries,
    dropped: state.dropped,
    hasMore: entries.length > 0 && entries.at(-1).sequence < state.next - 1,
    through: entries.length ? entries.at(-1).sequence : q.after,
  });
}
const started = performance.now();
let work = 0;
const budget = () => ++work <= 6000 && performance.now() - started < 100;
const visible = (el) => {
  if (!el?.isConnected || !el.getClientRects().length) return false;
  for (let p = el, depth = 0; p && depth < 64; p = p.parentElement, depth++) {
    const css = getComputedStyle(p);
    if (
      p.hidden ||
      p.inert ||
      css.visibility !== "visible" ||
      css.display === "none" ||
      Number(css.opacity) === 0
    )
      return false;
  }
  return true;
};
const privateField = (el) =>
  /^(password|hidden|file)$/i.test(el.type || "") ||
  /password|secret|token|csrf|credential|one.?time|cc-number|cc-csc/i.test(
    [el.name, el.id, el.autocomplete].join(" "),
  );
const text = (el) => {
  let value = "";
  const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  for (
    let n = walk.nextNode();
    n && value.length < 512 && budget();
    n = walk.nextNode()
  ) {
    if (
      n.parentElement?.closest(
        "input,textarea,select,[contenteditable],script,style,noscript,template",
      ) ||
      !visible(n.parentElement)
    )
      continue;
    value += " " + n.data.slice(0, 512 - value.length);
  }
  return value.replace(/\s+/g, " ").trim().slice(0, 512);
};
if (q.action === "snapshot") {
  const state = {
    document,
    url: location.href,
    navigation: q.navigationId,
    snapshot: q.snapshotId,
    refs: new Map(),
    created: performance.now(),
  };
  globalThis.__lomiAgentDomV1 = state;
  const result = {
    workspaceId: q.workspaceId,
    panelId: q.panelId,
    browserGeneration: q.browserGeneration,
    navigationId: q.navigationId,
    snapshotId: q.snapshotId,
    snapshotKind: "dom",
    frameId: "main",
    origin: location.origin,
    url: location.href,
    capturedAt: new Date().toISOString(),
    viewport: {
      width: innerWidth,
      height: innerHeight,
      deviceScaleFactor: devicePixelRatio,
    },
    elements: [],
    truncated: false,
    omittedFrames: 0,
    limitations: [
      "Top-level semantic DOM only; not a complete accessibility tree.",
      "All child frames and form values are omitted.",
      "References expire after 60 seconds or the next snapshot.",
    ],
  };
  const bytes = (value) =>
    new TextEncoder().encode(JSON.stringify(value)).length;
  if (bytes(result) + 256 > q.maxBytes) return fail("RESOURCE_EXHAUSTED");
  let used = bytes(result);
  const walker = document.createTreeWalker(
    document.documentElement,
    NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
  );
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (!budget() || result.elements.length >= q.maxNodes) {
      result.truncated = true;
      break;
    }
    const el = n.nodeType === Node.ELEMENT_NODE ? n : n.parentElement;
    if (
      !el ||
      (el.closest("script,style,noscript,template,textarea,select,option") &&
        !["TEXTAREA", "SELECT"].includes(el.tagName))
    )
      continue;
    if (el.tagName === "IFRAME" || el.tagName === "FRAME") {
      result.omittedFrames++;
      continue;
    }
    if (!visible(el)) continue;
    const editableRoot = el.closest("[contenteditable]");
    if (editableRoot && (editableRoot !== el || n.nodeType === Node.TEXT_NODE))
      continue;
    let role = "",
      name = "";
    if (n.nodeType === Node.TEXT_NODE) {
      if (el.closest("button,a,label,h1,h2,h3,h4,h5,h6,input,textarea,select"))
        continue;
      name = n.data.slice(0, 512).replace(/\s+/g, " ").trim();
      if (!name) continue;
      role = "text";
    } else {
      const tag = el.tagName;
      role =
        el.getAttribute("role")?.slice(0, 64) ||
        {
          BUTTON: "button",
          A: "link",
          INPUT:
            {
              checkbox: "checkbox",
              radio: "radio",
              range: "slider",
              button: "button",
              submit: "button",
              reset: "button",
            }[el.type] || "textbox",
          TEXTAREA: "textbox",
          SELECT: "combobox",
          LABEL: "label",
        }[tag] ||
        (/^H[1-6]$/.test(tag)
          ? "heading"
          : el.isContentEditable
            ? "textbox"
            : "");
      if (!role || (tag === "INPUT" && el.type === "hidden")) continue;
      name = el.getAttribute("aria-label")?.slice(0, 512) || "";
      if (!name && el.labels?.length) name = text(el.labels[0]);
      if (
        !name &&
        !el.isContentEditable &&
        !["INPUT", "TEXTAREA", "SELECT"].includes(tag)
      )
        name = text(el);
    }
    const ref = "e" + (result.elements.length + 1);
    const editable =
      !el.disabled &&
      !el.readOnly &&
      (["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) ||
        el.isContentEditable);
    const item = {
      elementRef: ref,
      role,
      name,
      enabled: !el.disabled && el.getAttribute("aria-disabled") !== "true",
      editable,
      checked: ["checkbox", "radio"].includes(el.type) ? el.checked : null,
      valueLength:
        !privateField(el) && ["INPUT", "TEXTAREA"].includes(el.tagName)
          ? el.value.length
          : null,
    };
    const size = bytes(item) + 1;
    if (used + size + 128 > q.maxBytes) {
      result.truncated = true;
      break;
    }
    used += size;
    state.refs.set(ref, n);
    result.elements.push(item);
  }
  if (bytes(result) > q.maxBytes) return fail("RESOURCE_EXHAUSTED");
  return JSON.stringify(result);
}
if (q.action === "interact") {
  const state = globalThis.__lomiAgentDomV1;
  const current = () =>
    !expired() &&
    state &&
    state.document === document &&
    state.url === location.href &&
    state.snapshot === q.snapshotId &&
    state.navigation === q.navigationId &&
    performance.now() - state.created <= 60000;
  if (!current()) return fail("STALE_SNAPSHOT");
  if (q.interaction.type === "scroll") {
    if (document.activeElement?.matches("iframe,frame"))
      return fail("SCOPE_DENIED");
    if (
      q.elementRef !== "viewport" ||
      !Number.isFinite(q.interaction.deltaX) ||
      !Number.isFinite(q.interaction.deltaY) ||
      Math.abs(q.interaction.deltaX) > 10000 ||
      Math.abs(q.interaction.deltaY) > 10000
    )
      return fail("RESOURCE_EXHAUSTED");
    effectMayHaveStarted = true;
    window.scrollBy({
      left: q.interaction.deltaX,
      top: q.interaction.deltaY,
      behavior: "instant",
    });
    return JSON.stringify({
      workspaceId: q.workspaceId,
      panelId: q.panelId,
      browserGeneration: q.browserGeneration,
      navigationId: q.navigationId,
      snapshotId: q.snapshotId,
      elementRef: q.elementRef,
      dispatched: true,
      inputMode: "synthetic_dom",
      valueLength: null,
      defaultAction: null,
      scrollPosition: { x: scrollX, y: scrollY },
    });
  }
  const node = state.refs.get(q.elementRef);
  if (
    !(node instanceof HTMLElement) ||
    !node.isConnected ||
    node.ownerDocument !== document
  )
    return fail("STALE_SNAPSHOT");
  const el = node;
  if (
    el.disabled ||
    (el.readOnly && q.interaction.type === "fill") ||
    el.getAttribute("aria-disabled") === "true"
  )
    return fail("TARGET_BUSY");
  if (el.matches("input[type=file], input[type=hidden], a[download]"))
    return fail("UNSUPPORTED_CAPABILITY");
  const actionable = () => {
    if (!visible(el)) return false;
    const r = el.getBoundingClientRect();
    const left = Math.max(0, r.left),
      top = Math.max(0, r.top);
    const right = Math.min(innerWidth, r.right),
      bottom = Math.min(innerHeight, r.bottom);
    if (left >= right || top >= bottom) return false;
    const hit = document.elementFromPoint(
      (left + right) / 2,
      (top + bottom) / 2,
    );
    return hit === el || el.contains(hit);
  };
  if (!actionable()) return fail("PANEL_NOT_RENDERABLE");
  let length = null;
  if (q.interaction.type === "key") {
    if (document.activeElement !== el || el.getRootNode() !== document)
      return fail("TARGET_BUSY");
    if (
      ![
        "Enter",
        "Tab",
        "Escape",
        "Backspace",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "Home",
        "End",
      ].includes(q.interaction.key)
    )
      return fail("UNSUPPORTED_CAPABILITY");
    effectMayHaveStarted = true;
    el.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: q.interaction.key,
        code: q.interaction.key,
        bubbles: true,
        cancelable: true,
      }),
    );
    if (!expired() && el.isConnected && el.ownerDocument === document)
      el.dispatchEvent(
        new KeyboardEvent("keyup", {
          key: q.interaction.key,
          code: q.interaction.key,
          bubbles: true,
          cancelable: true,
        }),
      );
  } else if (q.interaction.type === "click") {
    effectMayHaveStarted = true;
    el.focus({ preventScroll: true });
    if (!current() || !el.isConnected || !actionable())
      return fail("STALE_SNAPSHOT");
    el.click();
  } else if (q.interaction.type === "fill") {
    const value = q.interaction.text;
    if (
      typeof value !== "string" ||
      new TextEncoder().encode(value).length > 16384
    )
      return fail("RESOURCE_EXHAUSTED");
    const input = el instanceof HTMLInputElement;
    const textarea = el instanceof HTMLTextAreaElement;
    const select = el instanceof HTMLSelectElement;
    if (
      (input &&
        ![
          "text",
          "search",
          "email",
          "url",
          "tel",
          "password",
          "number",
        ].includes(el.type)) ||
      (!input && !textarea && !select && !el.isContentEditable)
    )
      return fail("UNSUPPORTED_CAPABILITY");
    if (select && el.options.length > 1000) return fail("RESOURCE_EXHAUSTED");
    if (
      select &&
      (el.multiple ||
        ![...el.options].some((o) => o.value === value && !o.disabled))
    )
      return fail("UNSUPPORTED_CAPABILITY");
    effectMayHaveStarted = true;
    el.focus({ preventScroll: true });
    if (!current() || !el.isConnected || !actionable())
      return fail("STALE_SNAPSHOT");
    if (document.activeElement !== el) return fail("TARGET_BUSY");
    if (input || textarea || select) {
      const prototype = input
        ? HTMLInputElement.prototype
        : textarea
          ? HTMLTextAreaElement.prototype
          : HTMLSelectElement.prototype;
      Object.getOwnPropertyDescriptor(prototype, "value").set.call(el, value);
    } else {
      el.textContent = value;
    }
    el.dispatchEvent(
      new InputEvent("input", {
        bubbles: true,
        inputType: "insertText",
        data: select ? null : value,
      }),
    );
    if (expired()) return fail("OUTCOME_UNKNOWN");
    el.dispatchEvent(new Event("change", { bubbles: true }));
    await Promise.resolve();
    const retained = input || textarea || select ? el.value : el.textContent;
    if (!el.isConnected || retained !== value) return fail("OUTCOME_UNKNOWN");
    length = retained.length;
  } else return fail("UNSUPPORTED_CAPABILITY");
  return JSON.stringify({
    workspaceId: q.workspaceId,
    panelId: q.panelId,
    browserGeneration: q.browserGeneration,
    navigationId: q.navigationId,
    snapshotId: q.snapshotId,
    elementRef: q.elementRef,
    dispatched: true,
    inputMode: "synthetic_dom",
    defaultAction: q.interaction.type === "key" ? false : null,
    scrollPosition: null,
    valueLength: length,
  });
}
return fail("UNSUPPORTED_CAPABILITY");
