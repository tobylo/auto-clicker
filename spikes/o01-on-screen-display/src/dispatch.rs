//! One shape for every Wayland event handler: `Dispatch::event` takes six
//! arguments, most of them unused; `dispatch!` implements it by handing a
//! handler the state and one `Heard` holding the rest.

use wayland_client::{Proxy, QueueHandle};

/// One event and what came with it.
pub struct Heard<'a, P: Proxy, D, S> {
    pub proxy: &'a P,
    pub event: P::Event,
    pub data: &'a D,
    pub qh: &'a QueueHandle<S>,
}

/// `dispatch!(State: Proxy, Data => handler)` implements
/// `Dispatch<Proxy, Data>` for `State` by calling
/// `handler(&mut State, Heard<Proxy, Data, State>)`.
#[macro_export]
macro_rules! dispatch {
    ($state:ty: $proxy:ty, $data:ty => $handler:path) => {
        impl wayland_client::Dispatch<$proxy, $data> for $state {
            fn event(
                state: &mut Self,
                proxy: &$proxy,
                event: <$proxy as wayland_client::Proxy>::Event,
                data: &$data,
                _: &wayland_client::Connection,
                qh: &wayland_client::QueueHandle<Self>,
            ) {
                $handler(
                    state,
                    $crate::dispatch::Heard {
                        proxy,
                        event,
                        data,
                        qh,
                    },
                )
            }
        }
    };
}
