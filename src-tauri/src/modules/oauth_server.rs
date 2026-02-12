use crate::modules::oauth;
use std::sync::{Mutex, OnceLock};
use tauri::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::sync::watch;

struct OAuthFlowState {
    auth_url: String,
    redirect_uri: String,
    cancel_tx: watch::Sender<bool>,
    code_rx: Option<oneshot::Receiver<Result<String, String>>>,
}

static OAUTH_FLOW_STATE: OnceLock<Mutex<Option<OAuthFlowState>>> = OnceLock::new();

fn get_oauth_flow_state() -> &'static Mutex<Option<OAuthFlowState>> {
    OAUTH_FLOW_STATE.get_or_init(|| Mutex::new(None))
}

fn oauth_success_html() -> &'static str {
    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
    <html>\
    <body style='font-family: sans-serif; text-align: center; padding: 50px; background: #0d1117; color: #fff;'>\
        <h1 style='color: #4ade80;'>✅ 授权成功!</h1>\
        <p>您可以关闭此窗口返回应用。</p>\
        <script>setTimeout(function() { window.close(); }, 2000);</script>\
    </body>\
    </html>"
}

fn oauth_fail_html() -> &'static str {
    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
    <html>\
    <body style='font-family: sans-serif; text-align: center; padding: 50px; background: #0d1117; color: #fff;'>\
        <h1 style='color: #f87171;'>❌ 授权失败</h1>\
        <p>未能获取授权 Code，请返回应用重试。</p>\
    </body>\
    </html>"
}

async fn ensure_oauth_flow_prepared(app_handle: &tauri::AppHandle) -> Result<String, String> {
    use tauri::Emitter;

    if let Ok(state) = get_oauth_flow_state().lock() {
        if let Some(s) = state.as_ref() {
            return Ok(s.auth_url.clone());
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("无法绑定本地端口: {}", e))?;

    let port = listener
        .local_addr()
        .map_err(|e| format!("无法获取本地端口: {}", e))?
        .port();

    let redirect_uri = format!("http://127.0.0.1:{}/oauth-callback", port);
    let auth_url = oauth::get_auth_url(&redirect_uri);

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (code_tx, code_rx) = oneshot::channel::<Result<String, String>>();

    let code_tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(code_tx)));
    let app_handle_clone = app_handle.clone();

    let tx = code_tx.clone();
    let mut rx = cancel_rx;
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = tokio::select! {
            res = listener.accept() => res.map_err(|e| format!("接受连接失败: {}", e)),
            _ = rx.changed() => Err("OAuth cancelled".to_string()),
        } {
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer).await;
            let request = String::from_utf8_lossy(&buffer);
            let code = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|path| Url::parse(&format!("http://127.0.0.1:{}{}", port, path)).ok())
                .and_then(|url| {
                    url.query_pairs()
                        .find(|(k, _)| k == "code")
                        .map(|(_, v)| v.into_owned())
                });

            let (result, response_html) = match code {
                Some(code) => (Ok(code), oauth_success_html()),
                None => (
                    Err("未能在回调中获取 Authorization Code".to_string()),
                    oauth_fail_html(),
                ),
            };
            let _ = stream.write_all(response_html.as_bytes()).await;
            let _ = stream.flush().await;

            if let Some(sender) = tx.lock().await.take() {
                let _ = app_handle_clone.emit("oauth-callback-received", ());
                let _ = sender.send(result);
            }
        }
    });

    if let Ok(mut state) = get_oauth_flow_state().lock() {
        *state = Some(OAuthFlowState {
            auth_url: auth_url.clone(),
            redirect_uri,
            cancel_tx,
            code_rx: Some(code_rx),
        });
    }

    let _ = app_handle.emit("oauth-url-generated", &auth_url);

    Ok(auth_url)
}

/// 预生成 OAuth URL
pub async fn prepare_oauth_url(app_handle: tauri::AppHandle) -> Result<String, String> {
    ensure_oauth_flow_prepared(&app_handle).await
}

/// 取消当前的 OAuth 流程
pub fn cancel_oauth_flow() {
    if let Ok(mut state) = get_oauth_flow_state().lock() {
        if let Some(s) = state.take() {
            let _ = s.cancel_tx.send(true);
        }
    }
}

/// 启动 OAuth 流程并等待回调
pub async fn start_oauth_flow(
    app_handle: tauri::AppHandle,
) -> Result<oauth::TokenResponse, String> {
    let auth_url = ensure_oauth_flow_prepared(&app_handle).await?;

    use tauri_plugin_opener::OpenerExt;
    app_handle
        .opener()
        .open_url(&auth_url, None::<String>)
        .map_err(|e| format!("无法打开浏览器: {}", e))?;

    let (code_rx, redirect_uri) = {
        let mut lock = get_oauth_flow_state()
            .lock()
            .map_err(|_| "OAuth 状态锁被污染".to_string())?;
        let Some(state) = lock.as_mut() else {
            return Err("OAuth 状态不存在".to_string());
        };
        let rx = state
            .code_rx
            .take()
            .ok_or_else(|| "OAuth 授权已在进行中".to_string())?;
        (rx, state.redirect_uri.clone())
    };

    let code = match code_rx.await {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("等待 OAuth 回调失败".to_string()),
    };

    if let Ok(mut lock) = get_oauth_flow_state().lock() {
        *lock = None;
    }

    oauth::exchange_code(&code, &redirect_uri).await
}

/// 完成 OAuth 流程（不打开浏览器）

pub async fn complete_oauth_flow(
    app_handle: tauri::AppHandle,
) -> Result<oauth::TokenResponse, String> {
    let _ = ensure_oauth_flow_prepared(&app_handle).await?;

    let (code_rx, redirect_uri) = {
        let mut lock = get_oauth_flow_state()
            .lock()
            .map_err(|_| "OAuth 状态锁被污染".to_string())?;
        let Some(state) = lock.as_mut() else {
            return Err("OAuth 状态不存在".to_string());
        };
        let rx = state
            .code_rx
            .take()
            .ok_or_else(|| "OAuth 授权已在进行中".to_string())?;
        (rx, state.redirect_uri.clone())
    };

    let code = match code_rx.await {
        Ok(Ok(code)) => code,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("等待 OAuth 回调失败".to_string()),
    };

    if let Ok(mut lock) = get_oauth_flow_state().lock() {
        *lock = None;
    }

    oauth::exchange_code(&code, &redirect_uri).await
}
