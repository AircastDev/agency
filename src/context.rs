use crate::{actor::Actor, addr::Addr, agency::Agency};
use tokio::{select, sync::mpsc};

pub struct Context<A>
where
    A: Actor,
{
    mailbox: mpsc::Receiver<A::Msg>,
    priority_mailbox: mpsc::UnboundedReceiver<A::Msg>,
    pub(crate) stopped: bool,
    addr: Addr<A>,
    pub agency: Agency,
}

impl<A> Context<A>
where
    A: Actor,
{
    pub(crate) fn new(agency: Agency) -> Self {
        let (priority_mailer, priority_mailbox) = mpsc::unbounded_channel();
        let (mailer, mailbox) = mpsc::channel(16);
        Self {
            mailbox,
            priority_mailbox,
            stopped: false,
            addr: Addr::new(mailer, priority_mailer),
            agency,
        }
    }

    /// Pull the next message off the stack, waiting if there are none
    pub async fn message(&mut self) -> A::Msg {
        select! {
            biased;
            Some(msg) = self.priority_mailbox.recv() => {
                msg
            }
            Some(msg) = self.mailbox.recv() => {
                msg
            }
            else => {
                unreachable!("mailboxes live at least as long as the context");
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
}
