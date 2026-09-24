// Shared deterministic pages for native WKWebView and DOM adapter regressions.
export function browserFramePage(path) {
  const style =
    "<style>body{font:14px sans-serif;margin:8px}iframe{display:block;width:620px;height:350px}input{width:160px}#nested{width:480px;height:150px}</style>";
  const controls = (name) =>
    `<label>${name}<input id="name" aria-label="${name}" onkeydown="this.dataset.key=event.key"></label><button id="save" onclick="document.querySelector('#result').textContent='Saved: '+document.querySelector('#name').value;this.dataset.count=String(Number(this.dataset.count||0)+1)">Save ${name}</button><p id="result">Ready ${name}</p><input type="password" value="frame-private-password"><input type="hidden" value="frame-private-hidden">`;
  if (path.startsWith("/depth/")) {
    const level = Number(path.split("/")[2]);
    return `<!doctype html>${style}<p>Depth ${level}</p>${level < 7 ? `<iframe src="/depth/${level + 1}"></iframe>` : ""}`;
  }
  if (path.startsWith("/nested"))
    return `<!doctype html>${style}${controls("Nested")}<div style="height:1800px"></div>`;
  if (path.startsWith("/child"))
    return `<!doctype html>${style}${controls("Child")}<iframe id="nested" src="/nested"></iframe><div style="height:1800px"></div>`;
  return `<!doctype html>${style}<h1>Native browser frames</h1>${controls("Main")}<iframe id="child" src="/child"></iframe><iframe id="opaque" sandbox="allow-scripts" srcdoc="<p>opaque-private-text</p>"></iframe><iframe id="local" srcdoc="<p>srcdoc-private-text</p>"></iframe><iframe id="hidden" style="display:none" src="/nested?hidden"></iframe>`;
}
