//! `writebook mcp`: Writebook books as MCP tools.
//!
//! On stdio it serves every instance in the inventory, with the person's own
//! sign-ins, to an MCP client on the same machine. With --serve it serves one
//! instance over HTTP, and every request must include a Writebook token. The
//! server sends that token only to that instance and never uses a stored
//! login, so a request without a token can't use anyone else's.

mod log;
mod tools;

use anyhow::{Context, Result, bail};
use std::time::Instant;
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

use crate::inventory::{self, Inventory};
use crate::web::Web;
use crate::{api, creds};

pub fn run(instance: Option<&str>, read_only: bool, serve: Option<&str>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    log::start();

    match serve {
        None if instance.is_some() => bail!("--instance only goes with --serve: on stdio, every instance you added is served"),
        None => runtime.block_on(serve_stdio(read_only)),
        Some(address) => {
            let host = inventory::host(instance)?;
            creds::ensure_private_connection(&host)?;
            runtime.block_on(serve_http(host, read_only, address))
        }
    }
}

async fn serve_stdio(read_only: bool) -> Result<()> {
    let writebook = tokio::task::spawn_blocking(move || -> Result<Writebook> {
        let inventory = Inventory::load()?;

        if inventory.instances.is_empty() {
            bail!("there are no instances to serve. Add one with `writebook instance add <url>`");
        }

        let mut can_write = false;
        for instance in inventory.instances.values() {
            can_write |= creds::can_write(&instance.url)?;
        }

        Ok(Writebook { origin: Origin::Inventory, read_only: read_only || !can_write })
    })
    .await??;

    log::serving("inventory", "stdio", None, writebook.read_only);
    writebook.serve(rmcp::transport::stdio()).await?.waiting().await?;
    Ok(())
}

async fn serve_http(host: String, read_only: bool, address: &str) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await.with_context(|| format!("could not listen on {address}"))?;
    let served = host.clone();
    let writebook = Writebook { origin: Origin::Served { host }, read_only };

    log::serving(&served, "http", Some(format!("http://{}/mcp", listener.local_addr()?)), read_only);

    // Allow any Host header. Host checks stop a web page from using a local
    // server's stored login, and this server doesn't store one.
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(Vec::<String>::new())
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let service: StreamableHttpService<Writebook, NeverSessionManager> =
        StreamableHttpService::new(move || Ok(writebook.clone()), Default::default(), config);
    let router = axum::Router::new().route_service("/mcp", service).layer(middleware::from_fn(require_token))
        .layer(middleware::from_fn(log_request));

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(())
}

async fn log_request(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let response = next.run(request).await;

    log::http_request(&method, response.status().as_u16(), started.elapsed());
    response
}

/// Answer 401 to a request without a token before it reaches MCP, which is
/// how MCP clients expect to be asked for one.
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
    origin: Origin,
    read_only: bool,
}

/// Which instances the tools reach, and whose token they use.
#[derive(Clone)]
enum Origin {
    /// Every instance in the inventory, with the sign-ins of whoever started
    /// `writebook mcp`. Tools take the instance as an argument.
    Inventory,
    /// One instance, with the token in each HTTP request's Authorization header.
    Served { host: String },
}

/// What a tool needs to talk to an instance as the caller.
pub struct Connection {
    host: String,
    bearer: Option<String>,
}

impl Connection {
    /// The instance a tool call names, from the inventory, or the default one.
    fn to_instance(instance: Option<&str>) -> Result<Connection> {
        let inventory = Inventory::load()?;
        let host = inventory.find(instance)?.1.url.clone();
        Ok(Connection { host, bearer: None })
    }

    /// Before a change, so a read-only sign-in is told why it can't.
    fn ensure_can_write(&self) -> Result<()> {
        match self.bearer {
            Some(_) => Ok(()),
            None => creds::ensure_can_write(&self.host),
        }
    }

    /// Reads always send a token. The CLI reads published books without one,
    /// but a tool that quietly did would answer as if a lost sign-in were fine.
    fn web(&self) -> Result<Web> {
        let token = match &self.bearer {
            Some(token) => token.clone(),
            None => match creds::signed_in_token(&self.host)? {
                Some(token) => token.value,
                None => bail!(
                    "you're not signed in to {}. Sign in with `{}`",
                    self.host,
                    inventory::sign_in_command(&self.host, &inventory::profile_for(&self.host)?)
                ),
            },
        };

        Ok(Web::new(&self.host, Some(token)))
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
        let books = match &self.origin {
            Origin::Inventory => "Books on your Writebook instances. list_instances lists them, and every other tool \
                                  takes one as instance, or uses the default one."
                .to_string(),
            Origin::Served { host } => format!("Books on the Writebook at {host}."),
        };
        let changes = if self.read_only {
            "Nothing here can change a book."
        } else {
            "To change a page, read it first: update_page needs the fingerprint read_page gave, so it never \
             overwrites a change made in between."
        };

        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("writebook", env!("CARGO_PKG_VERSION")))
            .with_instructions(format!(
                "{books} Find a book with list_books or search, see its pages with book_contents, and read them with \
                 read_page or read_book. {changes}"
            ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::list(self.read_only, matches!(self.origin, Origin::Inventory))))
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let served = match &self.origin {
            Origin::Inventory => None,
            Origin::Served { host } => match context.extensions.get::<Parts>().and_then(|parts| bearer(&parts.headers)) {
                Some(token) => Some(Connection { host: host.clone(), bearer: Some(token) }),
                None => return Ok(failure("a Writebook token is required").into()),
            },
        };

        let read_only = self.read_only;
        let name = request.name.to_string();
        let arguments = request.arguments.unwrap_or_default();

        let started = Instant::now();
        let outcome = tokio::task::spawn_blocking(move || {
            let outcome = tools::call(&name, &arguments, served, read_only);
            log::tool_call(&name, &outcome, started.elapsed());
            outcome
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;

        Ok(match (outcome, &self.origin) {
            (Ok(value), _) => CallToolResult::success(vec![ContentBlock::text(serde_json::to_string_pretty(&value).unwrap_or_default())]),
            // The CLI's message for a rejected token tells you to sign in
            // again, which doesn't apply to a token sent over HTTP.
            (Err(error), Origin::Served { host }) if error.downcast_ref::<api::TokenRejected>().is_some() => {
                failure(&format!("{host} did not accept your Writebook token"))
            }
            (Err(error), _) => failure(&format!("{error:#}")),
        }
        .into())
    }
}

fn failure(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_string())])
}
