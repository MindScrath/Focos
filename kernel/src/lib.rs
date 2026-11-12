#![no_std]

//! 内核组件与平台接口；通用逻辑和平台布局校验可在开发机验证。

pub mod arch;
pub mod block;
pub mod mm;
pub mod platform;
