//! MCP input to a presented frame in the retained production Android canvas.
use super::*;

pub(super) async fn measure(
    wire: &mut Wire,
    main: &Webview,
    base: &Value,
    directory: &Path,
) -> Result<Value, String> {
    // The default native fixture has already qualified 720x1280 portrait
    // hardware and input sequences 1..8. This point lies in its color target.
    let install = format!(
        r#"
const {{AndroidCanvas}}=await import('/src/android/canvas.ts');
if(window.__mcpLatency)throw Error('Latency probe already installed');
const original=AndroidCanvas.prototype.draw;
const probe={{last:null,pending:null,samples:[],geometry:null,restore(){{AndroidCanvas.prototype.draw=original;delete window.__mcpLatency;}}}};
window.__mcpLatency=probe;
AndroidCanvas.prototype.draw=function(frame,...args){{
  original.call(this,frame,...args);
  if(this.element.closest('[data-android-pane-id]')?.getAttribute('data-android-pane-id')!=={})return;
  const gl=this.element.getContext('webgl');const pixel=new Uint8Array(4);
  gl.readPixels(Math.floor(this.element.width/2),Math.floor(this.element.height*.35),1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixel);
  const color=pixel[0]<20?0:pixel[0]>235?1:null;
  if(color===null)return;
  probe.geometry={{width:this.element.width,height:this.element.height,sourceWidth:frame.width,sourceHeight:frame.height,rotation:frame.rotation}};
  probe.last=color;
  const pending=probe.pending;
  if(pending&&!pending.drawn&&color!==pending.previous){{
    pending.drawn=true;
    requestAnimationFrame(()=>requestAnimationFrame(()=>{{
      if(probe.pending===pending){{probe.samples.push(performance.now()-pending.started);probe.pending=null;}}
    }}));
  }}
}};
return true;"#,
        base["panelId"]
    );
    javascript(main, &install).await?;
    let result = measure_samples(wire, main, base).await;
    let restore = javascript(main, "window.__mcpLatency?.restore();return true;").await;
    restore?;
    let result = result?;
    std::fs::write(
        directory.join("android-latency.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    if result["p95Ms"].as_f64().is_none_or(|p95| p95 > 150.0) {
        return Err(format!(
            "Android input-to-presented-image budget exceeded: {result}"
        ));
    }
    Ok(result)
}

async fn measure_samples(wire: &mut Wire, main: &Webview, base: &Value) -> Result<Value, String> {
    let mut sequence = 9;
    for sample in 0..54 {
        if sample > 0 {
            javascript(main,"const p=window.__mcpLatency;if(p.last===null||p.pending)throw Error('Latency sample not ready');p.pending={previous:p.last,started:performance.now(),drawn:false};return true;").await?;
        }
        for phase in ["down", "up"] {
            wire.android_packet(base,sequence,json!({"type":"touch","space":"hardware_display","identifier":0,"x":360,"y":832,"phase":phase})).await?;
            sequence += 1;
        }
        let mut ready = false;
        for _ in 0..100 {
            let state = javascript(
                main,
                "const p=window.__mcpLatency;return p.last!==null&&p.pending===null;",
            )
            .await?;
            if state == true {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if !ready {
            return Err("No guest color transition reached the presented canvas".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut result=javascript(main,"const p=window.__mcpLatency;return {samplesMs:p.samples.slice(3),warmupSamplesMs:p.samples.slice(0,3),geometry:p.geometry,visibility:document.visibilityState,focused:document.hasFocus(),devicePixelRatio};").await?;
    let mut samples = result["samplesMs"]
        .as_array()
        .ok_or("Missing latency samples")?
        .iter()
        .map(|v| v.as_f64().ok_or("Invalid latency"))
        .collect::<Result<Vec<_>, _>>()?;
    if samples.len() != 50 || result["visibility"] != "visible" || result["focused"] != true {
        return Err("Incomplete foreground latency measurement".into());
    }
    samples.sort_by(f64::total_cmp);
    result["p95Ms"] = json!(samples[47]);
    result["maxMs"] = json!(samples[49]);
    result["budgetMs"] = json!(150);
    result["endpoint"]=json!("Main performance.now before stdio MCP input through the second requestAnimationFrame after the changed guest pixel is drawn into the production WebGL canvas");
    result["hardwareDisplay"] = json!([720, 1280]);
    result["firstInputSequence"] = json!(9);
    result["nextInputSequence"] = json!(sequence);
    Ok(result)
}
