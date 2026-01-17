#![no_std]

pub mod animation;
pub mod apps;
pub mod http;
pub mod metrics;
pub mod nvs;
pub mod proto;
pub mod state;
pub mod tasks;
pub mod time;
pub mod wifi;

#[macro_export]
macro_rules! mk_static {
    ($t:ty) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit();
        x
    }};
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

#[macro_export]
macro_rules! fallible_task {
    ($name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty) => {
        #[::embassy_executor::task]
        pub async fn $name($($arg: $ty),*) {
            paste::paste! {
                [<$name _impl>]($($arg),*)
                    .await
                    .expect(concat!(stringify!($name), " failed"));
            }
        }
    };
    (pub $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty) => {
        #[::embassy_executor::task]
        pub async fn $name($($arg: $ty),*) {
            paste::paste! {
                [<$name _impl>]($($arg),*)
                    .await
                    .expect(concat!(stringify!($name), " failed"));
            }
        }
    };
}
