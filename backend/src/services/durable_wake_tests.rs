//! Crash injection drops the executing future only after the durable boundary.
use std::{future::Future, sync::Arc};
use tokio::sync::Notify;
tokio::task_local! { static CRASH: (&'static str, Arc<Notify>, Option<Arc<Notify>>); }

pub(crate) async fn after(site: &'static str) {
    if let Ok((wanted, hit, resume)) = CRASH.try_with(Clone::clone)
        && wanted == site
    {
        hit.notify_one();
        match resume {
            Some(resume) => resume.notified().await,
            None => std::future::pending::<()>().await,
        }
    }
}

pub(crate) async fn drop_after(site: &'static str, work: impl Future) {
    let hit = Arc::new(Notify::new());
    CRASH.scope((site, hit.clone(), None), async move {
        tokio::select! {
            _ = work => panic!("crash boundary was not reached: {site}"),
            () = hit.notified() => {},
            () = tokio::time::sleep(std::time::Duration::from_secs(30)) => panic!("crash boundary timed out: {site}"),
        }
    }).await;
}

pub(crate) async fn resume_after(
    site: &'static str,
    work: impl Future,
    change: impl Future<Output = ()>,
) {
    let hit = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    CRASH.scope((site,hit.clone(),Some(resume.clone())), async move {
        tokio::pin!(work);
        tokio::select! {
            _ = &mut work => panic!("boundary was not reached: {site}"),
            () = hit.notified() => {},
            () = tokio::time::sleep(std::time::Duration::from_secs(30)) => panic!("boundary timed out: {site}"),
        }
        change.await;
        resume.notify_one();
        work.await;
    }).await;
}

pub(crate) type Commands = Arc<std::sync::Mutex<std::collections::BTreeMap<String, usize>>>;

pub(crate) async fn monitored(db: &mongodb::Database) -> (mongodb::Database, Commands) {
    monitored_with_stop_polls(db, false).await
}

pub(crate) async fn monitored_with_stop_polls(
    db: &mongodb::Database,
    count_polls: bool,
) -> (mongodb::Database, Commands) {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands: Commands = Arc::new(std::sync::Mutex::new(Default::default()));
    let observed = commands.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            let collection = event.command.get_str(&event.command_name).unwrap_or("");
            let key = if count_polls && event.command.get_str("comment") == Ok("stop_poll") {
                "stop_poll".to_owned()
            } else {
                format!("{}:{collection}", event.command_name)
            };
            *observed.lock().unwrap().entry(key).or_default() += 1;
        }
    }));
    (
        mongodb::Client::with_options(options)
            .unwrap()
            .database(db.name()),
        commands,
    )
}

pub(crate) fn take(commands: &Commands) -> std::collections::BTreeMap<String, usize> {
    std::mem::take(&mut *commands.lock().unwrap())
}
