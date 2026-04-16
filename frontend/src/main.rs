use gloo_net::http::Request;
use gloo_timers::future::sleep;
use shared::{AppSocket, ClientMsg, HealthResponse, ServerMsg};
use std::time::Duration;
use wasm_bindgen_futures::spawn_local;
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

#[function_component(Home)]
fn home() -> Html {
    let health = use_state(|| None::<String>);
    let ws_status = use_state(|| "Connecting...".to_string());
    let ws_messages = use_state(Vec::<String>::new);

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

    // WebSocket connection via ws-bridge
    {
        let ws_status = ws_status.clone();
        let ws_messages = ws_messages.clone();
        use_effect_with((), move |_| {
            match ws_bridge::yew_client::connect::<AppSocket>() {
                Ok(conn) => {
                    ws_status.set("Connected".to_string());
                    let (mut tx, mut rx) = conn.split();

                    // Ping loop — sends a Ping every 5 seconds
                    spawn_local(async move {
                        loop {
                            sleep(Duration::from_secs(5)).await;
                            if tx.send(ClientMsg::Ping).await.is_err() {
                                break;
                            }
                        }
                    });

                    // Receive loop — updates UI state on each message
                    let msgs = ws_messages;
                    let status = ws_status;
                    spawn_local(async move {
                        while let Some(result) = rx.recv().await {
                            match result {
                                Ok(ServerMsg::Heartbeat) => {
                                    let mut current = (*msgs).clone();
                                    current.push("Received: Heartbeat".to_string());
                                    if current.len() > 10 {
                                        current.drain(..current.len() - 10);
                                    }
                                    msgs.set(current);
                                }
                                Ok(ServerMsg::Error { message }) => {
                                    let mut current = (*msgs).clone();
                                    current.push(format!("Error: {}", message));
                                    msgs.set(current);
                                }
                                Ok(ServerMsg::JobAccepted { job_id }) => {
                                    let mut current = (*msgs).clone();
                                    current.push(format!("Job accepted: {}", job_id));
                                    msgs.set(current);
                                }
                                Ok(ServerMsg::JobProgress {
                                    job_id,
                                    status: job_status,
                                }) => {
                                    let mut current = (*msgs).clone();
                                    current.push(format!("Job {}: {:?}", job_id, job_status));
                                    msgs.set(current);
                                }
                                Ok(ServerMsg::JobCompleted { job_id, result }) => {
                                    let mut current = (*msgs).clone();
                                    current.push(format!(
                                        "Job {} solved: RA={:.4} Dec={:.4}",
                                        job_id, result.ra_deg, result.dec_deg
                                    ));
                                    msgs.set(current);
                                }
                                Ok(ServerMsg::ServerShutdown { reason, .. }) => {
                                    status.set(format!("Server shutting down: {}", reason));
                                    break;
                                }
                                Err(e) => {
                                    status.set(format!("WebSocket error: {}", e));
                                    break;
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    ws_status.set(format!("Connect failed: {}", e));
                }
            }
        });
    }

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

            <div class="upload-area">
                <p>{ "Drag & drop an image here, or click to upload" }</p>
                <p style="color: #666; font-size: 0.9em;">
                    { "Supports FITS, JPEG, PNG, TIFF" }
                </p>
            </div>

            <div class="status" style="margin-left: 1rem;">
                { format!("WebSocket: {}", *ws_status) }
            </div>

            <div class="job-list">
                <h3>{ "Activity" }</h3>
                <ul>
                    { for (*ws_messages).iter().map(|m| html! { <li>{ m }</li> }) }
                </ul>
            </div>
        </div>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
