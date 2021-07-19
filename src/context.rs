use crate::{actor::Actor, addr::Addr, agency::Agency};
use std::collections::VecDeque;
use tokio::sync::mpsc::{self, channel};

pub struct Context<A>
where
    A: Actor,
{
    mailbox: mpsc::Receiver<A::Msg>,
    priority: VecDeque<A::Msg>,
    pub(crate) stopped: bool,
    addr: Addr<A>,
    pub agency: Agency,
}

impl<A> Context<A>
where
    A: Actor,
{
    pub(crate) fn new(agency: Agency) -> Self {
        let (sender, receiver) = channel(16);
        Self {
            mailbox: receiver,
            priority: VecDeque::new(),
            stopped: false,
            addr: Addr::new(sender),
            agency,
        }
    }

    /// Pull the next message off the stack, waiting if there are none
    pub async fn message(&mut self) -> A::Msg {
        if let Some(msg) = self.priority.pop_back() {
            return msg;
        }

        self.mailbox
            .recv()
            .await
            .expect("channel cannot be closed whilst context lives")
    }

    pub fn stop(&mut self) {
        self.stopped = true;
    }

    pub fn address(&self) -> Addr<A> {
        self.addr.clone()
    }

    /// Send a message back to this actor.
    ///
    /// Messages sent this way take priority over regular messages.
    pub fn notify(&mut self, msg: impl Into<A::Msg>) {
        self.priority.push_front(msg.into());
    }
}
