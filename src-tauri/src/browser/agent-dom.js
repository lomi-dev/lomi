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
  const walk = el.ownerDocument.createTreeWalker(el, NodeFilter.SHOW_TEXT);
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
    frames: new Map(),
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
    frames: [],
    elements: [],
    truncated: false,
    omittedFrames: 0,
    limitations: [
      "Semantic DOM of the main document and up to 15 same-origin HTTP(S) frames, depth at most 4; not a complete accessibility tree.",
      "Cross-origin, opaque, hidden, unsupported and over-budget frames and all form values are omitted.",
      "References expire after 60 seconds or the next snapshot.",
    ],
  };
  const bytes = (value) =>
    new TextEncoder().encode(JSON.stringify(value)).length;
  if (bytes(result) + 256 > q.maxBytes) return fail("RESOURCE_EXHAUSTED");
  let used = bytes(result);
  const visit = (doc, frameId, parent, host, depth) => {
    const win = doc.defaultView;
    const url = win.location.href;
    const context = { doc, win, url, frameId, parent, host };
    const viewportRef = frameId === "main" ? "viewport" : frameId + "-viewport";
    const frame = {
      frameId,
      parentFrameId: parent?.frameId ?? null,
      origin: win.location.origin,
      url,
      viewportRef,
      viewport: {
        width: win.innerWidth,
        height: win.innerHeight,
        deviceScaleFactor: win.devicePixelRatio,
      },
    };
    const size = bytes(frame) + 1;
    if (used + size + 128 > q.maxBytes) {
      result.truncated = true;
      return false;
    }
    used += size;
    result.frames.push(frame);
    state.frames.set(viewportRef, context);
    const walker = doc.createTreeWalker(
      doc.documentElement,
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
        let included = false;
        try {
          const child = el.contentDocument;
          const childWindow = child?.defaultView;
          if (
            visible(el) &&
            child?.documentElement &&
            childWindow.location.origin === q.origin &&
            /^https?:$/.test(childWindow.location.protocol) &&
            result.frames.length < 16 &&
            depth < 4
          ) {
            included = visit(
              child,
              "f" + result.frames.length,
              context,
              el,
              depth + 1,
            );
          }
        } catch {
          /* Opaque and cross-origin documents are never inspected. */
        }
        if (!included) result.omittedFrames++;
        continue;
      }
      if (!visible(el)) continue;
      const editableRoot = el.closest("[contenteditable]");
      if (
        editableRoot &&
        (editableRoot !== el || n.nodeType === Node.TEXT_NODE)
      )
        continue;
      let role = "",
        name = "";
      if (n.nodeType === Node.TEXT_NODE) {
        if (
          el.closest("button,a,label,h1,h2,h3,h4,h5,h6,input,textarea,select")
        )
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
                file: "file_input",
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
      const ref = frameId + "-e" + (result.elements.length + 1);
      const editable =
        !el.disabled &&
        !el.readOnly &&
        (["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) ||
          el.isContentEditable);
      const item = {
        elementRef: ref,
        frameId,
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
      state.refs.set(ref, { node: n, context });
      result.elements.push(item);
    }
    return true;
  };
  visit(document, "main", null, null, 0);
  if (bytes(result) > q.maxBytes) return fail("RESOURCE_EXHAUSTED");
  return JSON.stringify(result);
}
if (["interact", "upload_prepare", "upload"].includes(q.action)) {
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
  const reference = state.refs.get(q.elementRef);
  const context =
    q.interaction.type === "scroll"
      ? state.frames.get(q.elementRef)
      : reference?.context;
  const frameCurrent = (frame) => {
    try {
      for (let f = frame; f; f = f.parent) {
        if (
          f.win.document !== f.doc ||
          f.win.location.href !== f.url ||
          f.win.location.origin !== q.origin ||
          (f.host &&
            (!f.host.isConnected ||
              f.host.contentDocument !== f.doc ||
              f.host.ownerDocument !== f.parent.doc))
        )
          return false;
      }
      return !!frame;
    } catch {
      return false;
    }
  };
  if (!frameCurrent(context)) return fail("STALE_SNAPSHOT");
  const { doc, win } = context;
  const hitPoint = (el, d, w) => {
    if (!visible(el)) return false;
    const r = el.getBoundingClientRect();
    const left = Math.max(0, r.left),
      top = Math.max(0, r.top);
    const right = Math.min(w.innerWidth, r.right),
      bottom = Math.min(w.innerHeight, r.bottom);
    if (left >= right || top >= bottom) return false;
    const point = { x: (left + right) / 2, y: (top + bottom) / 2 };
    const hit = d.elementFromPoint(point.x, point.y);
    return hit === el || el.contains(hit) ? point : null;
  };
  const ancestorsVisible = (point) => {
    for (let f = context; f?.host; f = f.parent) {
      if (!visible(f.host)) return false;
      // Coordinate projection is qualified for axis-aligned, untransformed frames.
      for (let p = f.host, depth = 0; p; p = p.parentElement) {
        if (++depth > 64 || getComputedStyle(p).transform !== "none")
          return false;
      }
      const r = f.host.getBoundingClientRect();
      if (!f.host.offsetWidth || !f.host.offsetHeight) return false;
      point = {
        x:
          r.left +
          ((f.host.clientLeft + point.x) * r.width) / f.host.offsetWidth,
        y:
          r.top +
          ((f.host.clientTop + point.y) * r.height) / f.host.offsetHeight,
      };
      if (f.parent.doc.elementFromPoint(point.x, point.y) !== f.host)
        return false;
    }
    return true;
  };
  const stillCurrent = () => current() && frameCurrent(context);
  if (q.interaction.type === "scroll") {
    if (!ancestorsVisible({ x: win.innerWidth / 2, y: win.innerHeight / 2 }))
      return fail("PANEL_NOT_RENDERABLE");
    if (doc.activeElement?.matches("iframe,frame")) return fail("SCOPE_DENIED");
    if (
      !Number.isFinite(q.interaction.deltaX) ||
      !Number.isFinite(q.interaction.deltaY) ||
      Math.abs(q.interaction.deltaX) > 10000 ||
      Math.abs(q.interaction.deltaY) > 10000
    )
      return fail("RESOURCE_EXHAUSTED");
    effectMayHaveStarted = true;
    win.scrollBy({
      left: q.interaction.deltaX,
      top: q.interaction.deltaY,
      behavior: "instant",
    });
    if (!stillCurrent()) return fail("OUTCOME_UNKNOWN");
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
      scrollPosition: { x: win.scrollX, y: win.scrollY },
    });
  }
  const el = reference.node;
  if (
    !(el instanceof win.HTMLElement) ||
    !el.isConnected ||
    el.ownerDocument !== doc
  )
    return fail("STALE_SNAPSHOT");
  if (
    el.disabled ||
    (el.readOnly && q.interaction.type === "fill") ||
    el.getAttribute("aria-disabled") === "true"
  )
    return fail("TARGET_BUSY");
  const actionable = () => {
    const point = hitPoint(el, doc, win);
    return stillCurrent() && point && ancestorsVisible(point);
  };
  if (!actionable()) return fail("PANEL_NOT_RENDERABLE");
  if (q.action === "upload_prepare" || q.action === "upload") {
    if (!el.matches("input[type=file]") || context.frameId !== q.frameId)
      return fail("UNSUPPORTED_CAPABILITY");
    const target = {
      frameId: context.frameId,
      origin: win.location.origin,
      documentUrl: context.url,
      label: (el.getAttribute("aria-label") || (el.labels?.length ? text(el.labels[0]) : "")).slice(0, 512),
    };
    if (q.action === "upload_prepare") return JSON.stringify(target);
    if (target.documentUrl !== q.documentUrl)
      return fail("STALE_SNAPSHOT");
    if (
      typeof q.fileName !== "string" || !q.fileName || q.fileName.length > 255 ||
      /[\x00-\x1f\x7f/\\]/.test(q.fileName) ||
      !Number.isInteger(q.byteLength) || q.byteLength < 0 || q.byteLength > 4194304 ||
      typeof q.base64 !== "string" || q.base64.length > 5592408
    ) return fail("RESOURCE_EXHAUSTED");
    const binary = atob(q.base64);
    if (binary.length !== q.byteLength) return fail("REVISION_CONFLICT");
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    const transfer = new win.DataTransfer();
    transfer.items.add(new win.File([bytes], q.fileName, {type: q.mediaType, lastModified: 0}));
    const setter = Object.getOwnPropertyDescriptor(win.HTMLInputElement.prototype, "files")?.set;
    if (!setter || !stillCurrent() || !actionable()) return fail("STALE_SNAPSHOT");
    effectMayHaveStarted = true;
    setter.call(el, transfer.files);
    for (const type of ["input", "change"]) {
      if (!stillCurrent() || !el.isConnected || el.ownerDocument !== doc || el.type !== "file")
        return fail("OUTCOME_UNKNOWN");
      el.dispatchEvent(new win.Event(type, {bubbles: true}));
    }
    if (!stillCurrent()) return fail("OUTCOME_UNKNOWN");
    return JSON.stringify({dispatched: true, frameId: context.frameId});
  }
  if (el.matches("input[type=file], input[type=hidden], a[download]"))
    return fail("UNSUPPORTED_CAPABILITY");
  let length = null;
  if (q.interaction.type === "key") {
    if (doc.activeElement !== el || el.getRootNode() !== doc)
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
    for (let f = context; f?.host; f = f.parent)
      if (f.parent.doc.activeElement !== f.host) return fail("TARGET_BUSY");
    effectMayHaveStarted = true;
    el.dispatchEvent(
      new win.KeyboardEvent("keydown", {
        key: q.interaction.key,
        code: q.interaction.key,
        bubbles: true,
        cancelable: true,
      }),
    );
    if (stillCurrent() && el.isConnected && el.ownerDocument === doc)
      el.dispatchEvent(
        new win.KeyboardEvent("keyup", {
          key: q.interaction.key,
          code: q.interaction.key,
          bubbles: true,
          cancelable: true,
        }),
      );
  } else if (q.interaction.type === "click") {
    effectMayHaveStarted = true;
    el.focus({ preventScroll: true });
    if (!stillCurrent() || !el.isConnected || !actionable())
      return fail("STALE_SNAPSHOT");
    el.click();
  } else if (q.interaction.type === "fill") {
    const value = q.interaction.text;
    if (
      typeof value !== "string" ||
      new TextEncoder().encode(value).length > 16384
    )
      return fail("RESOURCE_EXHAUSTED");
    const input = el instanceof win.HTMLInputElement;
    const textarea = el instanceof win.HTMLTextAreaElement;
    const select = el instanceof win.HTMLSelectElement;
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
    if (!stillCurrent() || !el.isConnected || !actionable())
      return fail("STALE_SNAPSHOT");
    if (doc.activeElement !== el) return fail("TARGET_BUSY");
    if (input || textarea || select) {
      const prototype = input
        ? win.HTMLInputElement.prototype
        : textarea
          ? win.HTMLTextAreaElement.prototype
          : win.HTMLSelectElement.prototype;
      Object.getOwnPropertyDescriptor(prototype, "value").set.call(el, value);
    } else {
      el.textContent = value;
    }
    el.dispatchEvent(
      new win.InputEvent("input", {
        bubbles: true,
        inputType: "insertText",
        data: select ? null : value,
      }),
    );
    if (expired()) return fail("OUTCOME_UNKNOWN");
    el.dispatchEvent(new win.Event("change", { bubbles: true }));
    await Promise.resolve();
    const retained = input || textarea || select ? el.value : el.textContent;
    if (!stillCurrent() || !el.isConnected || retained !== value)
      return fail("OUTCOME_UNKNOWN");
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
