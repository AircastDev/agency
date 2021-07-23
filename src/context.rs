use crate::{actor::Actor, addr::Addr, agency::Agency};
use std::marker::PhantomData;
use tokio::{select, sync::mpsc};
use tokio_stream::{
    wrappers::{ReceiverStream, UnboundedReceiverStream},
    StreamExt,
};

pub struct Running;
pub struct Stopped;

pub trait Phase {}
impl Phase for Running {}
impl Phase for Stopped {}

pub struct Context<A: Actor, P: Phase = Running> {
    mailbox: ReceiverStream<A::Msg>,
    priority_mailbox: UnboundedReceiverStream<A::Msg>,
    pub(crate) stopped: bool,
    addr: Addr<A>,
    pub agency: Agency,
    _phase: PhantomData<P>,
}

impl<A: Actor> Context<A, Running> {
    pub(crate) fn new(agency: Agency) -> Self {
        let (priority_mailer, priority_mailbox) = mpsc::unbounded_channel();
        let (mailer, mailbox) = mpsc::channel(16);
        Self {
            mailbox: ReceiverStream::new(mailbox),
            priority_mailbox: UnboundedReceiverStream::new(priority_mailbox),
            stopped: false,
            addr: Addr::new(mailer, priority_mailer),
            agency,
            _phase: PhantomData,
        }
    }

    /// Pull the next message off the stack, waiting if there are none
    pub async fn message(&mut self) -> A::Msg {
        select! {
            biased;
            Some(msg) = self.priority_mailbox.next() => {
                msg
            }
            Some(msg) = self.mailbox.next() => {
                msg
            }
            else => {
                unreachable!("mailboxes live at least as long as the running context");
            }
        }
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
        self.addr
            .send_priority(msg)
            .expect("mailboxes live at least as long as the context");
    }

    pub(crate) fn next_phase(mut self) -> Context<A, Stopped> {
        self.mailbox.close();
        self.priority_mailbox.close();
        Context {
            mailbox: self.mailbox,
            priority_mailbox: self.priority_mailbox,
            stopped: true,
            addr: self.addr,
            agency: self.agency,
            _phase: PhantomData,
        }
    }
}

impl<A: Actor> Context<A, Stopped> {
    /// Collect all of the remaining, unhandled messages
    pub async fn drain(self) -> Vec<A::Msg> {
        self.priority_mailbox.chain(self.mailbox).collect().await
    }
}
