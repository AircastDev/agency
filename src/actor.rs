use crate::context::Context;
use async_trait::async_trait;

pub enum StoppingResult {
    Recover,
    Stop,
}

#[async_trait]
pub trait Actor: Send + Sync + Sized {
    type Msg: 'static + Send + Sync;

    async fn run(&mut self, ctx: &mut Context<Self>);

    async fn init(&mut self, _ctx: &mut Context<Self>) {}

    /// Called after ctx.stop() is called.
    ///
    /// Can be used to restart try and recover the actor and restart the run loop.
    async fn stopping(&mut self, _ctx: &mut Context<Self>) -> StoppingResult {
        StoppingResult::Stop
    }

    async fn stopped(self, _ctx: &mut Context<Self>) {}
}

#[async_trait]
pub trait Setup: Actor {
    type Args: Send + Sync;

    async fn setup(ctx: &mut Context<Self>, args: Self::Args) -> Option<Self>;
}
