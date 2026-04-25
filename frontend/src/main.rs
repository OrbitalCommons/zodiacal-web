use gloo_net::http::Request;
use gloo_timers::future::sleep;
use shared::{
    AppSocket, ClientMsg, HealthResponse, ServerMsg, SolveServerMsg, SolveSocket, SubmitJobResponse,
};
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlInputElement, ProgressEvent, XmlHttpRequest};
use yew::prelude::*;
use yew_router::prelude::*;

#[derive(Clone, Routable, PartialEq)]
enum Route {
    #[at("/")]
    Home,
    #[not_found]
    #[at("/404")]
    NotFound,
}

fn switch(route: Route) -> Html {
    match route {
        Route::Home => html! { <Home /> },
        Route::NotFound => html! { <h1>{ "404 - Not Found" }</h1> },
    }
}

#[function_component(App)]
pub fn app() -> Html {
    html! {
        <BrowserRouter>
            <Switch<Route> render={switch} />
        </BrowserRouter>
    }
}

/// Upload a file via XMLHttpRequest with progress tracking.
/// Returns the response text on success.
async fn upload_with_progress(
    form: web_sys::FormData,
    on_progress: impl Fn(f64) + 'static,
) -> Result<String, String> {
    let (tx, rx) = futures_channel::oneshot::channel::<Result<String, String>>();
    let tx = std::rc::Rc::new(std::cell::RefCell::new(Some(tx)));

    let xhr = XmlHttpRequest::new().map_err(|e| format!("{e:?}"))?;
    xhr.open("POST", "/api/upload")
        .map_err(|e| format!("{e:?}"))?;

    // Progress callback
    let upload = xhr.upload().map_err(|e| format!("{e:?}"))?;
    let progress_cb = Closure::wrap(Box::new(move |e: ProgressEvent| {
        if e.length_computable() {
            let pct = e.loaded() / e.total();
            on_progress(pct);
        }
    }) as Box<dyn FnMut(ProgressEvent)>);
    upload.set_onprogress(Some(progress_cb.as_ref().unchecked_ref()));
    progress_cb.forget();

    // Load callback (success)
    let tx_load = tx.clone();
    let xhr_load = xhr.clone();
    let load_cb = Closure::wrap(Box::new(move || {
        let text = xhr_load.response_text().ok().flatten().unwrap_or_default();
        if let Some(tx) = tx_load.borrow_mut().take() {
            let _ = tx.send(if xhr_load.status().unwrap_or(0) == 200 {
                Ok(text)
            } else {
                Err(format!("HTTP {}", xhr_load.status().unwrap_or(0)))
            });
        }
    }) as Box<dyn FnMut()>);
    xhr.set_onload(Some(load_cb.as_ref().unchecked_ref()));
    load_cb.forget();

    // Error callback
    let tx_err = tx;
    let error_cb = Closure::wrap(Box::new(move || {
        if let Some(tx) = tx_err.borrow_mut().take() {
            let _ = tx.send(Err("Network error".to_string()));
        }
    }) as Box<dyn FnMut()>);
    xhr.set_onerror(Some(error_cb.as_ref().unchecked_ref()));
    error_cb.forget();

    xhr.send_with_opt_form_data(Some(&form))
        .map_err(|e| format!("{e:?}"))?;

    rx.await.map_err(|_| "Upload cancelled".to_string())?
}

#[function_component(Home)]
fn home() -> Html {
    let health = use_state(|| None::<String>);
    let ws_status = use_state(|| "Connected".to_string());
    let solve_status = use_state(|| None::<String>);
    let upload_pct = use_state(|| None::<f64>);
    let activity = use_state(Vec::<String>::new);
    let file_input_ref = use_node_ref();
    // Optional hint inputs (text fields, parsed at submit). Empty string => omitted.
    let scale_min_ref = use_node_ref();
    let scale_max_ref = use_node_ref();
    let ra_hint_ref = use_node_ref();
    let dec_hint_ref = use_node_ref();
    let radius_hint_ref = use_node_ref();

    // Health check via HTTP
    {
        let health = health.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                match Request::get("/api/health").send().await {
                    Ok(resp) => {
                        if let Ok(data) = resp.json::<HealthResponse>().await {
                            health.set(Some(data.status));
                        }
                    }
                    Err(e) => health.set(Some(format!("Error: {}", e))),
                }
            });
        });
    }

    // Background AppSocket heartbeat
    {
        let ws_status = ws_status.clone();
        use_effect_with((), move |_| {
            match ws_bridge::yew_client::connect::<AppSocket>() {
                Ok(conn) => {
                    let (mut tx, mut rx) = conn.split();
                    spawn_local(async move {
                        loop {
                            sleep(Duration::from_secs(5)).await;
                            if tx.send(ClientMsg::Ping).await.is_err() {
                                break;
                            }
                        }
                    });
                    spawn_local(async move {
                        while let Some(result) = rx.recv().await {
                            match result {
                                Ok(ServerMsg::ServerShutdown { .. }) | Err(_) => break,
                                _ => {}
                            }
                        }
                    });
                }
                Err(e) => {
                    ws_status.set(format!("WS failed: {}", e));
                }
            }
        });
    }

    // Progress bar HTML
    let progress_bar = if let Some(pct) = *upload_pct {
        let width = format!("{}%", (pct * 100.0) as u32);
        let label = if pct >= 1.0 {
            "Processing...".to_string()
        } else {
            format!("{}%", (pct * 100.0) as u32)
        };
        html! {
            <div class="progress-bar-bg">
                <div class="progress-bar-fill" style={format!("width: {}", width)}>
                    { label }
                </div>
            </div>
        }
    } else {
        html! {}
    };

    html! {
        <div>
            <h1>{ "Zodiacal — Plate Solver" }</h1>
            <p>{ "Upload an astronomy image to determine its sky coordinates." }</p>

            <div class="status">
                { match (*health).as_ref() {
                    Some(s) => format!("Backend: {}", s),
                    None => "Checking backend...".to_string(),
                }}
            </div>

            <div class="upload-area" onclick={
                let file_input_ref = file_input_ref.clone();
                Callback::from(move |_: MouseEvent| {
                    if let Some(input) = file_input_ref.cast::<HtmlInputElement>() {
                        input.click();
                    }
                })
            }>
                <p>{ "Drag & drop an image here, or click to upload" }</p>
                <p style="color: #666; font-size: 0.9em;">
                    { "Supports FITS, JPEG, PNG, TIFF" }
                </p>
                { progress_bar }
                { if let Some(status) = (*solve_status).as_ref() {
                    html! { <p style="color: #7eb8da; margin-top: 0.5rem;">{ status }</p> }
                } else {
                    html! {}
                }}
            </div>
            <details class="hints" style="margin-top: 1rem;">
                <summary style="cursor: pointer; color: #aaa;">
                    { "Optional hints (speeds up solving)" }
                </summary>
                <div class="hints-grid">
                    <label>
                        { "Scale min (\"/px)" }
                        <input ref={scale_min_ref.clone()} type="number" step="any" placeholder="e.g. 0.10" />
                    </label>
                    <label>
                        { "Scale max (\"/px)" }
                        <input ref={scale_max_ref.clone()} type="number" step="any" placeholder="e.g. 0.20" />
                    </label>
                    <label>
                        { "RA hint (deg)" }
                        <input ref={ra_hint_ref.clone()} type="number" step="any" placeholder="0–360" />
                    </label>
                    <label>
                        { "Dec hint (deg)" }
                        <input ref={dec_hint_ref.clone()} type="number" step="any" placeholder="-90 to 90" />
                    </label>
                    <label>
                        { "Search radius (deg)" }
                        <input ref={radius_hint_ref.clone()} type="number" step="any" placeholder="e.g. 5" />
                    </label>
                </div>
            </details>
            <input
                ref={file_input_ref.clone()}
                type="file"
                accept=".fits,.fit,.fts,.jpeg,.jpg,.png,.tiff,.tif"
                style="display: none;"
                onchange={
                    let solve_status = solve_status.clone();
                    let upload_pct = upload_pct.clone();
                    let activity = activity.clone();
                    let scale_min_ref = scale_min_ref.clone();
                    let scale_max_ref = scale_max_ref.clone();
                    let ra_hint_ref = ra_hint_ref.clone();
                    let dec_hint_ref = dec_hint_ref.clone();
                    let radius_hint_ref = radius_hint_ref.clone();
                    Callback::from(move |e: Event| {
                        let input: HtmlInputElement = e.target_unchecked_into();
                        let Some(files) = input.files() else { return };
                        let Some(file) = files.get(0) else { return };
                        let filename = file.name();
                        // Read hint inputs synchronously before moving into async
                        let read_hint = |r: &NodeRef| -> Option<String> {
                            r.cast::<HtmlInputElement>().and_then(|el| {
                                let v = el.value();
                                let trimmed = v.trim();
                                if trimmed.is_empty() {
                                    None
                                } else {
                                    Some(trimmed.to_string())
                                }
                            })
                        };
                        let scale_min = read_hint(&scale_min_ref);
                        let scale_max = read_hint(&scale_max_ref);
                        let ra_hint = read_hint(&ra_hint_ref);
                        let dec_hint = read_hint(&dec_hint_ref);
                        let radius_hint = read_hint(&radius_hint_ref);
                        let solve_status = solve_status.clone();
                        let upload_pct = upload_pct.clone();
                        let activity = activity.clone();
                        solve_status.set(Some(format!("Uploading {}...", filename)));
                        upload_pct.set(Some(0.0));
                        spawn_local(async move {
                            // Build FormData with the file + optional hint fields
                            let form = web_sys::FormData::new().unwrap();
                            let _ = form.append_with_blob_and_filename("file", &file, &filename);
                            if let Some(v) = scale_min { let _ = form.append_with_str("scale_min_arcsec", &v); }
                            if let Some(v) = scale_max { let _ = form.append_with_str("scale_max_arcsec", &v); }
                            if let Some(v) = ra_hint { let _ = form.append_with_str("ra_hint_deg", &v); }
                            if let Some(v) = dec_hint { let _ = form.append_with_str("dec_hint_deg", &v); }
                            if let Some(v) = radius_hint { let _ = form.append_with_str("radius_hint_deg", &v); }

                            // Upload with progress
                            let pct_state = upload_pct.clone();
                            let resp_text = match upload_with_progress(form, move |pct| {
                                pct_state.set(Some(pct));
                            })
                            .await
                            {
                                Ok(text) => text,
                                Err(e) => {
                                    solve_status.set(Some(format!("Upload failed: {}", e)));
                                    upload_pct.set(None);
                                    return;
                                }
                            };

                            upload_pct.set(None);

                            let data: SubmitJobResponse = match serde_json::from_str(&resp_text) {
                                Ok(d) => d,
                                Err(_) => {
                                    solve_status
                                        .set(Some("Upload failed: bad response".to_string()));
                                    return;
                                }
                            };

                            let job_id = data.job_id;
                            let job_short = &job_id.to_string()[..8];
                            solve_status.set(Some(format!("Job {} — connecting...", job_short)));

                            // Open solve WebSocket for progress
                            let window = web_sys::window().unwrap();
                            let location = window.location();
                            let protocol = location.protocol().unwrap();
                            let host = location.host().unwrap();
                            let ws_proto =
                                if protocol == "https:" { "wss:" } else { "ws:" };
                            let ws_url =
                                format!("{}//{}/ws/solve/{}", ws_proto, host, job_id);
                            let conn = match ws_bridge::yew_client::connect_to::<SolveSocket>(
                                &ws_url,
                            ) {
                                Ok(c) => c,
                                Err(e) => {
                                    solve_status
                                        .set(Some(format!("WS connect failed: {}", e)));
                                    return;
                                }
                            };

                            let (_tx, mut rx) = conn.split();

                            // Listen for progress
                            while let Some(result) = rx.recv().await {
                                match result {
                                    Ok(SolveServerMsg::Accepted { .. }) => {
                                        solve_status.set(Some(format!(
                                            "Job {} — accepted",
                                            job_short
                                        )));
                                    }
                                    Ok(SolveServerMsg::Extracting { n_sources }) => {
                                        let msg = match n_sources {
                                            Some(n) if n > 0 => format!(
                                                "Job {} — {} sources extracted",
                                                job_short, n
                                            ),
                                            _ => format!(
                                                "Job {} — extracting sources...",
                                                job_short
                                            ),
                                        };
                                        solve_status.set(Some(msg));
                                    }
                                    Ok(SolveServerMsg::Solving { n_verified }) => {
                                        solve_status.set(Some(format!(
                                            "Job {} — solving ({} verified)...",
                                            job_short, n_verified
                                        )));
                                    }
                                    Ok(SolveServerMsg::Solved { result }) => {
                                        let msg = format!(
                                            "Solved! RA={:.4} Dec={:.4} Scale={:.2}\"/px",
                                            result.ra_deg,
                                            result.dec_deg,
                                            result.pixel_scale_arcsec,
                                        );
                                        solve_status.set(Some(msg.clone()));
                                        let mut current = (*activity).clone();
                                        current.push(format!("{}: {}", filename, msg));
                                        activity.set(current);
                                        break;
                                    }
                                    Ok(SolveServerMsg::Failed { reason }) => {
                                        let msg = format!("Failed: {}", reason);
                                        solve_status.set(Some(msg.clone()));
                                        let mut current = (*activity).clone();
                                        current.push(format!("{}: {}", filename, msg));
                                        activity.set(current);
                                        break;
                                    }
                                    Err(e) => {
                                        solve_status
                                            .set(Some(format!("WS error: {}", e)));
                                        break;
                                    }
                                }
                            }
                        });
                    })
                }
            />

            <div class="status" style="margin-left: 1rem;">
                { &*ws_status }
            </div>

            <div class="job-list">
                <h3>{ "Results" }</h3>
                <ul>
                    { for (*activity).iter().map(|m| html! { <li>{ m }</li> }) }
                </ul>
            </div>
        </div>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
