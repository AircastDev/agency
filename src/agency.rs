use crate::{
    actor::{Actor, Setup, StoppingResult},
    addr::Addr,
    context::Context,
};
use futures_util::stream::FuturesUnordered;
use std::{fmt::Debug, future::Future};
use tokio::{
    select,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};
use tokio_stream::StreamExt;

pub struct AgencyHandle {
    futures: FuturesUnordered<JoinHandle<()>>,
    channel: (
        UnboundedSender<JoinHandle<()>>,
        UnboundedReceiver<JoinHandle<()>>,
    ),
}

impl AgencyHandle {
    fn new() -> Self {
        Self {
            futures: FuturesUnordered::new(),
            channel: unbounded_channel(),
        }
    }

    fn spawner(&self) -> Spawner {
        Spawner::new(self.channel.0.clone())
    }

    pub async fn wait(mut self) {
        loop {
            select! {
                biased;
                fut = self.channel.1.recv() => {
                    self.futures.push(fut.expect("sender is held by the handle"));
                }
                res = self.futures.next() => {
                    // TODO: i think we can catch and log panics here?
                    if res.is_none() {
                        return;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Spawner {
    sender: UnboundedSender<JoinHandle<()>>,
}

impl Spawner {
    fn new(sender: UnboundedSender<JoinHandle<()>>) -> Self {
        Self { sender }
    }

    fn spawn<T>(&self, fut: T)
    where
        T: Future<Output = ()> + Send + 'static,
    {
        let handle = tokio::task::spawn(fut);
        self.sender
            .send(handle)
            .expect("attempt to spawn task after the handler was dropped");
    }
}

#[derive(Clone)]
pub struct Agency {
    spawner: Spawner,
}

impl Agency {
    pub fn new() -> (Self, AgencyHandle) {
        let handle = AgencyHandle::new();
        (
            Agency {
                spawner: handle.spawner(),
            },
            handle,
        )
    }

    pub fn hire<A>(&self, mut actor: A) -> Addr<A>
    where
        A: 'static + Actor,
    {
        let mut ctx = Context::new(self.clone());
        let addr = ctx.address();
        self.spawner.spawn(async move {
            actor.init(&mut ctx).await;

            loop {
                while !ctx.stopped {
                    actor.run(&mut ctx).await;
                }

                match actor.stopping(&mut ctx).await {
                    StoppingResult::Recover => ctx.stopped = false,
                    StoppingResult::Stop => {
                        break;
                    }
                }
            }

            actor.stopped(ctx.next_phase()).await;
        });
        addr
    }

    pub fn hire_with<A>(&self, args: A::Args) -> Addr<A>
    where
        A: 'static + Setup,
    {
        let mut ctx = Context::new(self.clone());
        let addr = ctx.address();
        self.spawner.spawn(async move {
            if let Some(mut actor) = A::setup(&mut ctx, args).await {
                actor.init(&mut ctx).await;

                loop {
                    while !ctx.stopped {
                        actor.run(&mut ctx).await;
                    }

                    match actor.stopping(&mut ctx).await {
                        StoppingResult::Recover => ctx.stopped = false,
                        StoppingResult::Stop => {
                            break;
                        }
                    }
                }

                actor.stopped(ctx.next_phase()).await;
            }
        });
        addr
    }
}
