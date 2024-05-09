use std::time::Duration;
use std::{env, io};

use cached::instant::Instant;
use chrono::{DateTime, Utc};
use log::{debug, error, warn};
use oauth2::basic::{BasicClient, BasicTokenResponse};
use oauth2::reqwest::async_http_client;
use oauth2::url::Url;
use oauth2::{AuthUrl, ClientId, ClientSecret, Scope, TokenResponse, TokenUrl};
use sqlx::PgPool;

use crate::calendar::models::Event;

pub(in crate::calendar) struct APIRequestor {
    client: reqwest::Client,
    pool: PgPool,
    oauth_token: Option<BasicTokenResponse>,
}

impl From<&PgPool> for APIRequestor {
    fn from(pool: &PgPool) -> Self {
        let keep_alive = Duration::from_secs(2);
        let client = reqwest::Client::builder()
            .tcp_keepalive(keep_alive)
            .http2_keep_alive_while_idle(true)
            .http2_keep_alive_interval(keep_alive)
            .gzip(true)
            .zstd(true)
            .brotli(true)
            .deflate(true)
            .build()
            .expect("the request client builder is correctly configured");
        Self {
            client,
            pool: pool.clone(),
            oauth_token: None,
        }
    }
}

impl APIRequestor {
    pub(crate) async fn refresh(&mut self, id: &str) -> Result<(), crate::BoxedError> {
        let sync_start = Utc::now();
        let token = self.try_unwrap_or_refresh_token().await?;
        let start = Instant::now();
        let url = format!("https://campus.tum.de/tumonline/co/connectum/api/rooms/{id}/calendars");
        let events: Vec<Event> = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await?
            .json()
            .await?;
        debug!(
            "finished fetching for {id}: {cnt} calendar events in {elapsed:?}",
            cnt = events.len(),
            elapsed = start.elapsed()
        );
        let events = events
            .into_iter()
            .map(|mut e| {
                e.room_code = id.into();
                e
            })
            .collect::<Vec<Event>>();
        self.store(&events, &sync_start, id).await?;
        Ok(())
    }
    async fn try_unwrap_or_refresh_token(&mut self) -> Result<String, crate::BoxedError> {
        match &self.oauth_token {
            None => {
                debug!("oauth token not present");
                self.oauth_token = Some(Self::fetch_new_oauth_token().await?);
            }
            Some(token) => {
                let expires_in = token.expires_in();
                debug!("oauth expires in {expires_in:?}");
                if !expires_in.is_some_and(|e| e > Duration::from_secs(10)) {
                    self.oauth_token = Some(Self::fetch_new_oauth_token().await?);
                }
            }
        };

        Ok(self
            .oauth_token
            .as_ref()
            .expect("the token has been set in the last step")
            .access_token()
            .secret()
            .clone())
    }
}

impl APIRequestor {
    async fn store(
        &self,
        events: &[Event],
        last_calendar_scrape_at: &DateTime<Utc>,
        id: &str,
    ) -> Result<(), crate::BoxedError> {
        // insert into db
        let mut tx = self.pool.begin().await?;
        if let Err(e) = self.delete_events(&mut tx, id).await {
            error!("could not delete existing events because {e:?}");
            tx.rollback().await?;
            return Err(e.into());
        }
        for (i, event) in events.iter().enumerate() {
            // conflicts cannot occur because all values for said room were dropped
            if let Err(e) = event.store(&mut tx).await {
                warn!(
                    "ignoring insert {event:?} ({i}/{total}) because {e:?}",
                    total = events.len()
                );
            }
        }
        if let Err(e) = self
            .update_last_calendar_scrape_at(&mut tx, id, last_calendar_scrape_at)
            .await
        {
            error!("could not update last_calendar_scrape_at because {e:?}");
            tx.rollback().await?;
            return Err(e.into());
        }
        tx.commit().await?;
        debug!("finished inserting into the db for {id}");
        Ok(())
    }

    async fn fetch_new_oauth_token() -> Result<BasicTokenResponse, crate::BoxedError> {
        let client_id = env::var("CONNECTUM_OAUTH_CLIENT_ID")
            .map_err(|e| {
                error!("CONNECTUM_OAUTH_CLIENT_ID needs to be set: {e:?}");
                io::Error::other("please configure the environment variable CONNECTUM_OAUTH_CLIENT_ID to use this endpoint")
            })?
            .trim().into();
        let client_secret = env::var("CONNECTUM_OAUTH_CLIENT_SECRET")
            .map_err(|e| {
                error!("CONNECTUM_OAUTH_CLIENT_SECRET needs to be set: {e:?}");
                io::Error::other("please configure the environment variable CONNECTUM_OAUTH_CLIENT_SECRET to use this endpoint")
            })?
            .trim().into();

        // for urls see https://campus.tum.de/tumonline/co/public/sec/auth/realms/CAMPUSonline/.well-known/openid-configuration
        let auth_url = Url::parse("https://campus.tum.de/tumonline/co/public/sec/auth/realms/CAMPUSonline/protocol/openid-connect/auth")?;
        let token_url = Url::parse("https://campus.tum.de/tumonline/co/public/sec/auth/realms/CAMPUSonline/protocol/openid-connect/token")?;

        let token = BasicClient::new(
            ClientId::new(client_id),
            Some(ClientSecret::new(client_secret)),
            AuthUrl::from_url(auth_url),
            Some(TokenUrl::from_url(token_url)),
        )
        .exchange_client_credentials()
        .add_scope(Scope::new("connectum-rooms.read".into()))
        .request_async(async_http_client)
        .await;
        Ok(token?)
    }
    async fn delete_events(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &str,
    ) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
        sqlx::query!(r#"DELETE FROM calendar WHERE room_code = $1"#, id)
            .execute(&mut **tx)
            .await
    }
    async fn update_last_calendar_scrape_at(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &str,
        last_calendar_scrape_at: &DateTime<Utc>,
    ) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
        sqlx::query!(
            "UPDATE en SET last_calendar_scrape_at = $1 WHERE key=$2",
            last_calendar_scrape_at,
            id
        )
        .execute(&mut **tx)
        .await?;
        sqlx::query!(
            "UPDATE de SET last_calendar_scrape_at = $1 WHERE key=$2",
            last_calendar_scrape_at,
            id
        )
        .execute(&mut **tx)
        .await
    }
}