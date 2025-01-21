use anyhow::{anyhow, Result};
use lettre::{
    message::header::ContentType, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use log::{error, info};
use tokio::{
    select,
    sync::mpsc::{channel, Receiver, Sender},
    time::{Instant, Interval},
};

use crate::config::{Notification, NotificationConfig};

pub struct Aggregator {
    notify_rx: Receiver<Notification>,
    config: NotificationConfig,
    interval: Option<Interval>,
}

impl Aggregator {
    pub fn init(
        notify_config: NotificationConfig,
        interval: Option<Interval>,
    ) -> Sender<Notification> {
        let (notify_tx, notify_rx) = channel(1);

        let aggregator = Self {
            notify_rx,
            config: notify_config,
            interval,
        };
        tokio::spawn(aggregator.start());

        notify_tx
    }

    async fn start(self) -> Result<()> {
        let config = self.config;
        let mut notify_rx = self.notify_rx;
        let mut interval = self.interval;
        let mut queue = Vec::new();
        loop {
            select! {
                Some(notification) = notify_rx.recv() => {
                    info!("Received notification");
                    match interval {
                        None => Self::send(notification, &config).await?,
                        Some(_) => queue.push(notification),
                    }
                }
                Some(_) = Self::tick(&mut interval) => {
                    if queue.is_empty() {
                        info!("Tick...");
                        continue;
                    } else if queue.len() == 1 {
                        Self::send(queue.pop().unwrap(), &config).await?;
                        continue;
                    }

                    info!("Sending aggregate");
                    let body = queue.drain(..).map(|notification| notification.body).collect::<Vec<String>>().join("\n");
                    Self::send(Notification {
                        r#type: config.name.clone(),
                        title: "Ramon Aggregated Notification".into(),
                        body,
                    }, &config).await?;
                }
            }
        }
    }

    async fn send(notification: Notification, config: &NotificationConfig) -> Result<()> {
        info!("Sending notification '{}'", notification.title);

        if let Some(smtp) = &config.smtp {
            let email = Message::builder()
                .from(smtp.from.clone())
                .to(smtp.to.clone())
                .subject(&notification.title)
                .header(ContentType::TEXT_PLAIN)
                .body(notification.body.clone())
                .map_err(|err| anyhow!("Failed to build email: {err}"))?;
            let mailer = match &smtp.login {
                None => AsyncSmtpTransport::unencrypted_localhost(),
                Some(login) => {
                    let creds = Credentials::new(login.username.clone(), login.password.clone());
                    AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&login.host)
                        .map_err(|err| anyhow!("Failed to parse {:?}: {err}", login.host))?
                        .credentials(creds)
                        .build()
                }
            };
            if let Err(err) = mailer.send(email).await {
                error!("[{}] Failed to send email: {err}", config.name);
                if smtp.login.is_none() {
                    info!(
                        "[{}] Consider setting smtp_host, login, and password.",
                        config.name
                    );
                }
            }
        }

        Ok(())
    }

    async fn tick(interval: &mut Option<Interval>) -> Option<Instant> {
        match interval {
            None => None,
            Some(interval) => Some(interval.tick().await),
        }
    }
}
