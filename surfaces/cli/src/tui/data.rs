//! 界面数据：屏幕要显示什么。
//!
//! 数据由后台线程加载，通过消息总线送回主线程。主线程**只读**这些结构，
//! 不直接碰 Runtime —— 这样界面永远不会因为一次慢查询而卡住。

use else_protocol::api::{
    AttentionWindowSnapshot, EventResponse, PlanView, ResourceView, ScopeView, ThreadSnapshot,
};
use else_protocol::attention::AttentionRequest;
use else_protocol::connector::{ConnectorHealthReport, ConnectorRegistration};
use else_protocol::world::{IdentityWorld, PersonalBrief, ProjectWorld};

/// Overview 屏幕的数据：一天里真正要你管的事。
#[derive(Debug, Clone)]
pub struct OverviewData {
    pub brief: PersonalBrief,
    pub attention: Vec<AttentionWindowSnapshot>,
    pub threads: Vec<ThreadSnapshot>,
    pub projects: Vec<ProjectWorld>,
    pub identities: Vec<IdentityWorld>,
}

/// Thread 详情：目标、委托树、计划、资源账与事件流。
#[derive(Debug, Clone)]
pub struct ThreadDetailData {
    pub thread: ThreadSnapshot,
    pub scopes: Vec<ScopeView>,
    pub plan: PlanView,
    pub resources: ResourceView,
    pub events: Vec<EventResponse>,
}

/// 一个 Connector 加上它的健康状况。
#[derive(Debug, Clone)]
pub struct ConnectorRow {
    pub connector: ConnectorRegistration,
    pub health: ConnectorHealthReport,
}

/// 决策队列：窗口里的请求 + 没有进窗口但仍待裁决的请求。
///
/// 窗口是调度机制，不是用户要理解的东西。用户看到的是「有几件事在等我」。
#[derive(Debug, Clone)]
pub struct DecisionsData {
    pub windows: Vec<AttentionWindowSnapshot>,
    /// 窗口外但仍可裁决的请求，带上它们所属的 Thread。
    pub loose: Vec<(else_protocol::ids::ThreadId, AttentionRequest)>,
}

/// 当前屏幕的数据。
#[derive(Debug, Clone, Default)]
pub enum Data {
    #[default]
    Empty,
    Overview(Box<OverviewData>),
    Threads(Vec<ThreadSnapshot>),
    ThreadDetail(Box<ThreadDetailData>),
    Decisions(Box<DecisionsData>),
    Connectors(Vec<ConnectorRow>),
}
