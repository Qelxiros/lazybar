use std::{
    fs::File, io::Read, pin::Pin, rc::Rc, sync::Arc, task::Poll, time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use derive_builder::Builder;
use futures::{FutureExt, StreamExt};
use lazy_static::lazy_static;
use regex::Regex;
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT},
    Client,
};
use serde::Deserialize;
use tokio::{
    task::{self, JoinHandle},
    time::{interval, Interval},
};
use tokio_stream::Stream;

use crate::{
    bar::PanelDrawInfo,
    common::{draw_common, PanelCommon},
    remove_array_from_config, remove_bool_from_config,
    remove_string_from_config, remove_uint_from_config, PanelConfig,
};

lazy_static! {
    static ref REGEX: Regex =
        Regex::new(r#"<(?<url>\S*)>; rel="next""#).unwrap();
}

/// Displays the number of github notifications you have.
#[derive(Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Github {
    name: &'static str,
    #[builder(default = "Duration::from_secs(60)")]
    interval: Duration,
    token: String,
    #[builder(default = "Vec::new()")]
    filter: Vec<String>,
    #[builder(default)]
    include: bool,
    #[builder(default = "true")]
    show_zero: bool,
    format: &'static str,
    common: PanelCommon,
}

impl Github {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        height: i32,
        count: usize,
    ) -> Result<PanelDrawInfo> {
        let mut text = if !self.show_zero && count == 0 {
            String::new()
        } else {
            self.format.replace("%count%", count.to_string().as_str())
        };

        if count == 50 {
            text.push('+');
        }

        draw_common(
            cr,
            text.as_str(),
            &self.common.attrs[0],
            self.common.dependence,
            self.common.images.clone(),
            height,
        )
    }
}

#[async_trait(?Send)]
impl PanelConfig for Github {
    /// Configuration options:
    ///
    /// - `interval`: how long to wait between requests. The panel will never
    ///   poll more often than this, but it may poll less often according to the
    ///   `X-Poll-Interval` header of the reponse. See
    ///   <https://docs.github.com/en/rest/activity/notifications?apiVersion=2022-11-28#about-github-notifications>
    ///   for more information.
    /// - `token`: A file path containing your GitHub token. Visit <https://github.com/settings/tokens/new>
    ///   to generate a token. The `notifications` scope is required.
    /// - `filter`: An array of strings corresponding to notification reasons.
    ///   See <https://docs.github.com/en/rest/activity/notifications?apiVersion=2022-11-28#about-notification-reasons>
    ///   for details.
    /// - `include`: Whether to include or exclude the reasons in `filter`. If
    ///   `include` is true, only notifications with one of the reasons in
    ///   `filter` will be counted. Otherwise, only notifications with reasons
    ///   not in `filter` will be counted.
    /// - `show_zero`: Whether or not the panel is shown when you have zero
    ///   notifications.
    ///
    /// See [`PanelCommon::parse`].
    fn parse(
        name: &'static str,
        table: &mut std::collections::HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> anyhow::Result<Self> {
        let mut builder = GithubBuilder::default();

        builder.name(name);

        if let Some(interval) = remove_uint_from_config("interval", table) {
            builder.interval(Duration::from_secs(interval.max(1) * 60));
        }

        if let Some(path) = remove_string_from_config("token", table) {
            let mut token = String::new();
            File::open(path)?.read_to_string(&mut token)?;

            builder.token(token);
        }

        if let Some(filter) = remove_array_from_config("filter", table) {
            builder.filter(
                filter
                    .iter()
                    .filter_map(|v| v.clone().into_string().ok())
                    .collect(),
            );
        }

        if let Some(include) = remove_bool_from_config("include", table) {
            builder.include(include);
        }

        if let Some(show_zero) = remove_bool_from_config("show_zero", table) {
            builder.show_zero(show_zero);
        }

        let (common, formats) =
            PanelCommon::parse(table, &[""], &["%count%"], &[""], &[])?;

        builder.common(common);
        builder.format(formats.into_iter().next().unwrap().leak());

        Ok(builder.build()?)
    }

    fn props(&self) -> (&'static str, bool) {
        (self.name, self.common.visible)
    }

    async fn run(
        mut self: Box<Self>,
        cr: std::rc::Rc<cairo::Context>,
        global_attrs: crate::attrs::Attrs,
        height: i32,
    ) -> anyhow::Result<(
        crate::PanelStream,
        Option<
            crate::ipc::ChannelEndpoint<
                crate::bar::Event,
                crate::bar::EventResponse,
            >,
        >,
    )> {
        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        let stream = GithubStream::new(
            self.token.as_str(),
            self.interval,
            self.filter.clone(),
            self.include,
        )?
        .map(move |r| self.draw(&cr, height, r?));

        Ok((Box::pin(stream), None))
    }
}

struct GithubStream {
    handle: Option<JoinHandle<Result<usize>>>,
    interval: Arc<futures::lock::Mutex<Interval>>,
    filter: Vec<String>,
    include: bool,
    client: Client,
}

impl GithubStream {
    pub fn new(
        token: &str,
        duration: Duration,
        filter: Vec<String>,
        include: bool,
    ) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-Github-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static("lazybar"));
        let mut secret =
            HeaderValue::from_str(format!("Bearer {}", token.trim()).as_str())?;
        secret.set_sensitive(true);
        headers.insert(AUTHORIZATION, secret);
        let client = Client::builder().default_headers(headers).build()?;
        let interval = Arc::new(futures::lock::Mutex::new(interval(duration)));
        Ok(Self {
            handle: None,
            interval,
            filter,
            include,
            client,
        })
    }
}

impl Stream for GithubStream {
    type Item = Result<usize>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(ref mut handle) = &mut self.handle {
            let val = handle.poll_unpin(cx).map(Result::ok);

            if val.is_ready() {
                self.handle = None;
            }

            val
        } else {
            let interval = self.interval.clone();
            let filter = self.filter.clone();
            let include = self.include;
            let client = self.client.clone();
            self.handle = Some(task::spawn(get_notifications(
                interval, filter, include, client,
            )));

            Poll::Pending
        }
    }
}

async fn get_notifications(
    interval: Arc<futures::lock::Mutex<Interval>>,
    filter: Vec<String>,
    include: bool,
    client: Client,
) -> Result<usize> {
    interval.lock().await.tick().await;

    let request = client.get("https://api.github.com/notifications").build()?;

    let response = client.execute(request).await?;

    let headers = response.headers().clone();
    let wait = headers
        .get("X-Poll-Interval")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);

    interval.lock().await.reset_after(Duration::from_secs(wait));

    let body = response.json::<Vec<Thread>>().await?;

    let count = body
        .into_iter()
        .filter(|t| !(include ^ filter.contains(&t.reason)))
        .count();

    Ok(count)
}

#[derive(Deserialize, Debug)]
#[non_exhaustive]
struct Thread {
    reason: String,
}
