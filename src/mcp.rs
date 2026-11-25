//! `writebook mcp`: one Writebook's books as MCP tools.
//!
//! On stdio it serves the person running it, with their sign-in, for an MCP
//! client on the same machine. With --server it listens on HTTP for anyone,
//! and each request brings its own Writebook token, which is only ever sent
//! to the instance the server was started for. That server keeps no sign-in
//! of its own, so there is nothing on it to steal or borrow.

mod tools;

use anyhow::{Context, Result, bail};
use axum::extract::Request;
use axum::http::{StatusCode, header, request::Parts};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};

use crate::{api, creds, inventory};
use crate::web::Web;

pub fn run(instance: Option<&str>, read_only: bool, server: Option<&str>) -> Result<()> {
    let host = inventory::host(instance)?;
    creds::ensure_private_connection(&host)?;

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    match server {
        None => runtime.block_on(serve_stdio(host, read_only)),
        Some(address) => runtime.block_on(serve_http(host, read_only, address)),
    }
}

async fn serve_stdio(host: String, read_only: bool) -> Result<()> {
    let writebook = tokio::task::spawn_blocking(move || -> Result<Writebook> {
        if creds::existing_access_token(&host)?.is_none() {
            bail!(
                "writebook mcp serves your sign-in to {host}, and there isn't one. Sign in with `{}` first",
                inventory::sign_in_command(&host, &inventory::profile_for(&host)?)
            );
        }

        let read_only = read_only || !creds::can_write(&host)?;
        Ok(Writebook { host, read_only, tokens: Tokens::Stored })
    })
    .await??;

    eprintln!("Serving {} over MCP on stdio{}", writebook.host, if writebook.read_only { ", read-only" } else { "" });
    writebook.serve(rmcp::transport::stdio()).await?.waiting().await?;
    Ok(())
}

async fn serve_http(host: String, read_only: bool, address: &str) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await.with_context(|| format!("could not listen on {address}"))?;
    let writebook = Writebook { host, read_only, tokens: Tokens::Bearer };

    eprintln!(
        "Serving {} over MCP at http://{}/mcp{}. Every request needs its own Writebook token",
        writebook.host,
        listener.local_addr()?,
        if read_only { ", read-only" } else { "" }
    );

    // Any Host is let in: Host checks keep a web page from reaching a local
    // server's stored sign-in, and this server has none.
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(Vec::<String>::new())
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let service: StreamableHttpService<Writebook, NeverSessionManager> =
        StreamableHttpService::new(move || Ok(writebook.clone()), Default::default(), config);
    let router = axum::Router::new().route_service("/mcp", service).layer(middleware::from_fn(require_token));

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(())
}

/// Turn away a request without a token before it reaches MCP, the way an
/// MCP client expects to be told to bring one.
async fn require_token(request: Request, next: Next) -> Response {
    if bearer(request.headers()).is_some() {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Bearer")], "a Writebook token is required\n").into_response()
    }
}

fn bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer "))?.trim();
    Some(token.to_string()).filter(|it| !it.is_empty())
}

#[derive(Clone)]
struct Writebook {
    host: String,
    read_only: bool,
    tokens: Tokens,
}

/// Where a tool call's token comes from.
#[derive(Clone, Copy)]
enum Tokens {
    /// The sign-in of whoever started `writebook mcp`.
    Stored,
    /// The Authorization header of the HTTP request carrying the call.
    Bearer,
}

/// What a tool needs to talk to the instance as the caller.
pub struct Connection {
    host: String,
    bearer: Option<String>,
}

impl Connection {
    fn web(&self) -> Result<Web> {
        match &self.bearer {
            Some(token) => Ok(Web::new(&self.host, Some(token.clone()))),
            None => Ok(Web::new(&self.host, creds::existing_access_token(&self.host)?)),
        }
    }

    fn client(&self) -> Result<api::Client> {
        match &self.bearer {
            Some(token) => Ok(api::Client::with_token(&self.host, creds::Token::given(token.clone()))),
            None => api::Client::connect(&self.host),
        }
    }
}

impl ServerHandler for Writebook {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("writebook", env!("CARGO_PKG_VERSION")))
            .with_instructions(format!(
                "Books on the Writebook at {}. Find a book with list_books or search, see its pages with book_contents, \
                 and read them with read_page or read_book.{}",
                self.host,
                if self.read_only {
                    " Nothing here can change a book."
                } else {
                    " To change a page, read it first: update_page needs the fingerprint read_page gave, so it never \
                     overwrites a change made in between."
                }
            ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::list(self.read_only)))
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let bearer = match self.tokens {
            Tokens::Stored => None,
            Tokens::Bearer => match context.extensions.get::<Parts>().and_then(|parts| bearer(&parts.headers)) {
                Some(token) => Some(token),
                None => return Ok(failure("a Writebook token is required").into()),
            },
        };

        let connection = Connection { host: self.host.clone(), bearer };
        let read_only = self.read_only;
        let name = request.name.to_string();
        let arguments = request.arguments.unwrap_or_default();

        let outcome = tokio::task::spawn_blocking(move || tools::call(&name, &arguments, &connection, read_only))
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

        Ok(match outcome {
            Ok(value) => CallToolResult::success(vec![ContentBlock::text(serde_json::to_string_pretty(&value).unwrap_or_default())]),
            // The CLI's advice for a rejected token is about its own sign-in,
            // which isn't the one a caller over HTTP brought.
            Err(error) if matches!(self.tokens, Tokens::Bearer) && error.downcast_ref::<api::TokenRejected>().is_some() => {
                failure(&format!("{} did not accept your Writebook token", self.host))
            }
            Err(error) => failure(&format!("{error:#}")),
        }
        .into())
    }
}

fn failure(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_string())])
}
