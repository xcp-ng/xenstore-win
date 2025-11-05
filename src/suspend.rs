use std::io;

#[trait_variant::make(AsyncSuspend: Send)]
pub trait LocalAsyncSuspend {
    async fn register_suspend(
        &self,
    ) -> io::Result<impl futures::Stream<Item = ()> + Unpin + 'static>;
}
