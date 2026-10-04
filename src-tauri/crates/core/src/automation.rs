//! Versioned workflow definitions and effect-free execution rules.
//!
//! The composing host serializes mutations and persists the entire state before
//! dispatching effects. This module deliberately owns no scheduler or native API.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub enabled: bool,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    #[serde(default)]
    pub policy: WorkflowPolicy,
    #[serde(default)]
    pub scope: WorkflowScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowNode {
    pub id: String,
    pub label: String,
    pub position: Position,
    pub config: NodeConfig,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
    pub port: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_project_id: Option<String>,
    #[serde(default)]
    pub targets: Vec<WorkflowTarget>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTarget {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowPolicy {
    pub max_concurrent_runs: u32,
    pub require_merge_approval: bool,
    pub allow_writes: bool,
    /// Local mutation permission does not imply permission to create a commit.
    pub allow_commit: bool,
    /// Publishing requires this separate grant, even for an existing commit.
    pub allow_push: bool,
    /// The native publish gate binds approval to the checked publication state.
    /// Human approval does not itself grant commit or push authority.
    pub require_publish_approval: bool,
    pub max_retries: u32,
    pub max_agent_turns: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
}

impl Default for WorkflowPolicy {
    fn default() -> Self {
        Self {
            max_concurrent_runs: 1,
            require_merge_approval: true,
            allow_writes: false,
            allow_commit: false,
            allow_push: false,
            require_publish_approval: true,
            max_retries: 0,
            max_agent_turns: 1,
            max_cost_usd: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Baseline {
    #[default]
    IgnoreExisting,
    IncludeExisting,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConditionOperator {
    Equals,
    NotEquals,
    Exists,
    Truthy,
}

/// Saved argv for an approved validation command. Agents select the fixed
/// `run_checks` tool; they cannot provide or replace executable/argument values.
/// The native host separately enforces its executable allowlist and OS sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CheckCommand {
    pub executable: String,
    pub args: Vec<String>,
}

pub fn validate_check_commands(checks: &[CheckCommand]) -> Result<(), String> {
    if checks.len() > 8 {
        return Err("At most eight fixed check commands are allowed".into());
    }
    for check in checks {
        if check.executable.is_empty()
            || check.executable.len() > 128
            || matches!(check.executable.as_str(), "." | "..")
            || !check
                .executable
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        {
            return Err(
                "Check executable must be a bounded executable name, not a path or shell command"
                    .into(),
            );
        }
        if check.args.len() > 64 {
            return Err("A check may have at most 64 arguments".into());
        }
        if check.args.iter().any(|arg| {
            arg.len() > 4096
                || arg.chars().any(|character| {
                    character.is_control() || ['*', '?', '[', ']'].contains(&character)
                })
        }) {
            return Err("Check arguments must be bounded literal values without controls or wildcard patterns".into());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NodeConfig {
    Trigger {
        event: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interval_seconds: Option<u64>,
        #[serde(default)]
        baseline: Baseline,
    },
    Query {
        operation: String,
        version: u32,
        inputs: Value,
    },
    Condition {
        path: String,
        operator: ConditionOperator,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
    },
    Jev {
        operation: String,
        version: u32,
        inputs: Value,
    },
    Agent {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(default = "empty_object")]
        context: Value,
        #[serde(default)]
        tools: Vec<String>,
        #[serde(default)]
        checks: Vec<CheckCommand>,
        #[serde(default = "empty_object")]
        output_schema: Value,
    },
    Action {
        operation: String,
        version: u32,
        inputs: Value,
    },
    Approval {
        message: String,
    },
    Wait {
        seconds: u64,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    #[default]
    Query,
    Action,
    Jev,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Effect {
    #[default]
    Read,
    Write,
    Merge,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InputType {
    String,
    Number,
    Boolean,
    Object,
    Array,
    #[default]
    Any,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationDescriptor {
    pub id: String,
    pub version: u32,
    pub kind: OperationKind,
    pub title: String,
    #[serde(default)]
    pub required_inputs: Vec<String>,
    #[serde(default)]
    pub input_schema: BTreeMap<String, InputType>,
    pub output_ports: Vec<String>,
    pub effect: Effect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

fn empty_object() -> Value {
    json!({})
}

fn operation(config: &NodeConfig) -> Option<(&str, u32, &Value, OperationKind)> {
    match config {
        NodeConfig::Query {
            operation,
            version,
            inputs,
        } => Some((operation, *version, inputs, OperationKind::Query)),
        NodeConfig::Action {
            operation,
            version,
            inputs,
        } => Some((operation, *version, inputs, OperationKind::Action)),
        NodeConfig::Jev {
            operation,
            version,
            inputs,
        } => Some((operation, *version, inputs, OperationKind::Jev)),
        _ => None,
    }
}

fn descriptor<'a>(
    config: &NodeConfig,
    registry: &'a [OperationDescriptor],
) -> Option<&'a OperationDescriptor> {
    let (id, version, _, kind) = operation(config)?;
    registry
        .iter()
        .find(|op| op.id == id && op.version == version && op.kind == kind)
}

fn ports(config: &NodeConfig, registry: &[OperationDescriptor]) -> Vec<String> {
    if let Some(op) = descriptor(config, registry) {
        return op.output_ports.clone();
    }
    match config {
        NodeConfig::Condition { .. } => vec!["true", "false", "error"],
        NodeConfig::Jev { .. } | NodeConfig::Agent { .. } => vec!["next", "error", "uncertain"],
        _ => vec!["next", "error"],
    }
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn type_matches(value: &Value, kind: InputType) -> bool {
    match kind {
        InputType::String => value.is_string(),
        InputType::Number => value.is_number(),
        InputType::Boolean => value.is_boolean(),
        InputType::Object => value.is_object(),
        InputType::Array => value.is_array(),
        InputType::Any => true,
    }
}

pub fn validate_workflow(
    workflow: &Workflow,
    registry: &[OperationDescriptor],
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let mut issue = |code: &str, message: String, node_id: Option<&str>| {
        issues.push(ValidationIssue {
            code: code.into(),
            message,
            node_id: node_id.map(str::to_owned),
        })
    };
    if workflow.id.trim().is_empty() || workflow.name.trim().is_empty() {
        issue("required", "Workflow id and name are required".into(), None);
    }
    if workflow.policy.max_concurrent_runs == 0 || workflow.policy.max_agent_turns == 0 {
        issue(
            "policy",
            "Concurrency and agent turn limits must be positive".into(),
            None,
        );
    }
    if workflow
        .policy
        .max_cost_usd
        .is_some_and(|cost| !cost.is_finite() || cost <= 0.0)
    {
        issue(
            "budget",
            "USD budget must be positive and finite".into(),
            None,
        );
    }
    let mut targets = BTreeSet::new();
    for target in &workflow.scope.targets {
        if target.project_id.trim().is_empty() || !targets.insert(&target.project_id) {
            issue(
                "scope",
                "Selected project ids must be nonempty and unique".into(),
                None,
            );
        }
    }
    let mut ids = BTreeSet::new();
    let mut triggers = Vec::new();
    for node in &workflow.nodes {
        if node.id.trim().is_empty() || !ids.insert(node.id.as_str()) {
            issue(
                "node_id",
                "Node ids must be nonempty and unique".into(),
                Some(&node.id),
            );
        }
        if !node.position.x.is_finite() || !node.position.y.is_finite() {
            issue(
                "position",
                "Node positions must be finite".into(),
                Some(&node.id),
            );
        }
        match &node.config {
            NodeConfig::Trigger {
                event,
                interval_seconds,
                ..
            } => {
                triggers.push(node.id.as_str());
                if event.trim().is_empty() {
                    issue(
                        "required",
                        "Trigger event is required".into(),
                        Some(&node.id),
                    );
                }
                if interval_seconds.is_some_and(|s| s < 60) {
                    issue(
                        "interval",
                        "Polling interval must be at least 60 seconds".into(),
                        Some(&node.id),
                    );
                }
            }
            NodeConfig::Condition {
                path,
                operator,
                value,
            } => {
                if path.trim().is_empty() {
                    issue(
                        "required",
                        "Condition path is required".into(),
                        Some(&node.id),
                    );
                }
                if matches!(
                    operator,
                    ConditionOperator::Equals | ConditionOperator::NotEquals
                ) && value.is_none()
                {
                    issue(
                        "required",
                        "Comparison value is required".into(),
                        Some(&node.id),
                    );
                }
            }
            NodeConfig::Agent {
                prompt,
                tools,
                checks,
                output_schema,
                ..
            } => {
                if prompt.trim().is_empty() {
                    issue(
                        "required",
                        "Agent prompt is required".into(),
                        Some(&node.id),
                    );
                }
                if tools.iter().any(|tool| {
                    !["read_file", "list_files", "write_file", "run_checks"]
                        .contains(&tool.as_str())
                }) {
                    issue(
                        "agent_tool",
                        "Agent tool is not supported by the restricted broker".into(),
                        Some(&node.id),
                    );
                }
                if tools.iter().any(|tool| tool == "run_checks") && checks.is_empty() {
                    issue(
                        "agent_checks",
                        "run_checks requires at least one saved check command".into(),
                        Some(&node.id),
                    );
                }
                if let Err(error) = validate_check_commands(checks) {
                    issue("agent_checks", error, Some(&node.id));
                }
                if let Err(error) = validate_output_schema(output_schema) {
                    issue("agent_schema", error, Some(&node.id));
                }
            }
            NodeConfig::Approval { message } if message.trim().is_empty() => issue(
                "required",
                "Approval message is required".into(),
                Some(&node.id),
            ),
            NodeConfig::Wait { seconds: 0 } => issue(
                "required",
                "Wait duration must be positive".into(),
                Some(&node.id),
            ),
            _ => {}
        }
        if let Some((id, version, inputs, kind)) = operation(&node.config) {
            if let Some(op) = descriptor(&node.config, registry) {
                if kind != OperationKind::Action && op.effect != Effect::Read {
                    issue(
                        "effect",
                        "Only action operations may write".into(),
                        Some(&node.id),
                    );
                }
                if !inputs.is_object() {
                    issue(
                        "input_type",
                        "Operation inputs must be an object".into(),
                        Some(&node.id),
                    );
                }
                if let Some(fields) = inputs.as_object() {
                    for key in fields.keys() {
                        if !op.input_schema.contains_key(key) {
                            issue(
                                "unknown_input",
                                format!("Input {key} is not declared by the operation"),
                                Some(&node.id),
                            );
                        }
                    }
                }
                for key in &op.required_inputs {
                    if inputs.get(key).is_none_or(|v| {
                        v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty())
                    }) {
                        issue(
                            "required_input",
                            format!("Input {key} is required"),
                            Some(&node.id),
                        );
                    }
                }
                for (key, expected) in &op.input_schema {
                    if let Some(value) = inputs.get(key) {
                        // References are checked again after resolution by the execution host.
                        if value.get("$ref").and_then(Value::as_str).is_none()
                            && !type_matches(value, *expected)
                        {
                            issue(
                                "input_type",
                                format!("Input {key} has the wrong type"),
                                Some(&node.id),
                            );
                        }
                    }
                }
            } else {
                issue(
                    "operation",
                    format!("Unknown or incompatible operation {id}@{version}"),
                    Some(&node.id),
                );
            }
        }
    }
    if triggers.len() != 1 {
        issue(
            "trigger",
            "A workflow requires exactly one trigger".into(),
            None,
        );
    }
    let mut edges = BTreeSet::new();
    for edge in &workflow.edges {
        if !ids.contains(edge.from.as_str()) || !ids.contains(edge.to.as_str()) {
            issue(
                "dangling_edge",
                "Edge references a missing node".into(),
                None,
            );
            continue;
        }
        if !edges.insert((&edge.from, &edge.to, &edge.port)) {
            issue("duplicate_edge", "Duplicate edge".into(), None);
        }
        if triggers.contains(&edge.to.as_str()) {
            issue(
                "trigger_input",
                "Trigger nodes cannot have incoming edges".into(),
                Some(&edge.to),
            );
        }
        let source = workflow.nodes.iter().find(|n| n.id == edge.from).unwrap();
        if !ports(&source.config, registry).contains(&edge.port) {
            issue(
                "port",
                format!("Unknown output port {}", edge.port),
                Some(&edge.from),
            );
        }
    }
    if topological_order(workflow).is_none() {
        issue(
            "cycle",
            "Workflow must be a directed acyclic graph".into(),
            None,
        );
    }
    if let Some(trigger) = triggers.first() {
        let mut reachable = BTreeSet::from([*trigger]);
        loop {
            let before = reachable.len();
            for edge in &workflow.edges {
                if reachable.contains(edge.from.as_str()) {
                    reachable.insert(edge.to.as_str());
                }
            }
            if before == reachable.len() {
                break;
            }
        }
        for node in &workflow.nodes {
            if !reachable.contains(node.id.as_str()) {
                issue(
                    "unreachable",
                    "Node is not reachable from the trigger".into(),
                    Some(&node.id),
                );
            }
        }
    }
    issues
}

fn topological_order(workflow: &Workflow) -> Option<Vec<&WorkflowNode>> {
    let mut incoming: BTreeMap<&str, usize> =
        workflow.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    for edge in &workflow.edges {
        if let Some(count) = incoming.get_mut(edge.to.as_str()) {
            *count += 1;
        }
    }
    let mut queue: VecDeque<&str> = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut ordered = Vec::new();
    while let Some(id) = queue.pop_front() {
        if let Some(node) = workflow.nodes.iter().find(|n| n.id == id) {
            ordered.push(node);
        }
        for edge in workflow.edges.iter().filter(|edge| edge.from == id) {
            if let Some(count) = incoming.get_mut(edge.to.as_str()) {
                *count -= 1;
                if *count == 0 {
                    queue.push_back(&edge.to);
                }
            }
        }
    }
    (ordered.len() == workflow.nodes.len()).then_some(ordered)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SimulationFixture {
    #[serde(default)]
    pub event: Value,
    #[serde(default)]
    pub outputs: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SimulationStep {
    pub node_id: String,
    pub status: SimulationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    pub input: Value,
    pub output: Value,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SimulationStatus {
    Simulated,
    Skipped,
    Error,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SimulationResult {
    pub steps: Vec<SimulationStep>,
    pub outputs: BTreeMap<String, Value>,
    pub effects_suppressed: bool,
}

/// Dot paths begin with `event` or `nodes.<node id>`; JSON pointers are also accepted.
pub fn lookup_value<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.starts_with('/') {
        return root.pointer(path);
    }
    path.split('.').try_fold(root, |value, part| {
        if let Some(array) = value.as_array() {
            part.parse::<usize>()
                .ok()
                .and_then(|index| array.get(index))
        } else {
            value.get(part)
        }
    })
}

pub fn resolve_value(value: &Value, context: &Value) -> Result<Value, String> {
    if let Some(reference) = value.get("$ref").and_then(Value::as_str) {
        if value.as_object().is_none_or(|object| object.len() != 1) {
            return Err("Reference objects may contain only $ref".into());
        }
        return lookup_value(context, reference)
            .cloned()
            .ok_or_else(|| format!("Missing reference {reference}"));
    }
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| Ok((key.clone(), resolve_value(value, context)?)))
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(Value::Object),
        Value::Array(items) => items
            .iter()
            .map(|value| resolve_value(value, context))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        _ => Ok(value.clone()),
    }
}

/// Shared by the native interpreter and simulation; missing data never becomes a
/// successful negative comparison. Truthy intentionally means the boolean true.
pub fn evaluate_condition(
    context: &Value,
    path: &str,
    operator: ConditionOperator,
    expected: Option<&Value>,
) -> Result<bool, String> {
    let actual = lookup_value(context, path);
    if matches!(operator, ConditionOperator::Exists) {
        return Ok(actual.is_some_and(|value| !value.is_null()));
    }
    let actual = actual.ok_or_else(|| format!("Missing condition path {path}"))?;
    match operator {
        ConditionOperator::Equals => Ok(actual == expected.ok_or("Comparison value is required")?),
        ConditionOperator::NotEquals => {
            Ok(actual != expected.ok_or("Comparison value is required")?)
        }
        ConditionOperator::Truthy => Ok(actual == &Value::Bool(true)),
        ConditionOperator::Exists => unreachable!("handled above"),
    }
}

/// The simulator traverses selected branches, using fixtures for observations.
/// No closure or effect port exists here, so simulation cannot dispatch work.
pub fn simulate(
    workflow: &Workflow,
    registry: &[OperationDescriptor],
    fixture: &SimulationFixture,
) -> Result<SimulationResult, String> {
    let issues = validate_workflow(workflow, registry);
    if !issues.is_empty() {
        return Err(format!(
            "invalid_workflow: {}",
            issues
                .iter()
                .map(|i| i.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let mut result = SimulationResult {
        steps: Vec::new(),
        outputs: BTreeMap::new(),
        effects_suppressed: true,
    };
    let mut selected: BTreeMap<String, String> = BTreeMap::new();
    for node in topological_order(workflow).expect("validated DAG") {
        let incoming: Vec<_> = workflow.edges.iter().filter(|e| e.to == node.id).collect();
        let active = matches!(node.config, NodeConfig::Trigger { .. })
            || incoming
                .iter()
                .any(|e| selected.get(&e.from) == Some(&e.port));
        if !active {
            result.steps.push(SimulationStep {
                node_id: node.id.clone(),
                status: SimulationStatus::Skipped,
                port: None,
                input: Value::Null,
                output: Value::Null,
                message: "Branch not selected".into(),
            });
            continue;
        }
        let context = json!({"event": fixture.event, "nodes": result.outputs});
        let mut step = SimulationStep {
            node_id: node.id.clone(),
            status: SimulationStatus::Simulated,
            port: Some("next".into()),
            input: Value::Null,
            output: Value::Null,
            message: String::new(),
        };
        match &node.config {
            NodeConfig::Trigger { .. } => {
                step.output = fixture.event.clone();
                step.message = "Provided event".into();
            }
            NodeConfig::Condition {
                path,
                operator,
                value,
            } => {
                step.input = lookup_value(&context, path).cloned().unwrap_or(Value::Null);
                match evaluate_condition(&context, path, *operator, value.as_ref()) {
                    Ok(branch) => {
                        step.port = Some(branch.to_string());
                        step.output = Value::Bool(branch);
                        step.message = format!("Selected {branch} branch");
                    }
                    Err(error) => {
                        step.status = SimulationStatus::Error;
                        step.port = Some("error".into());
                        step.message = error;
                    }
                }
            }
            NodeConfig::Query { inputs, .. }
            | NodeConfig::Jev { inputs, .. }
            | NodeConfig::Action { inputs, .. } => match resolve_value(inputs, &context) {
                Err(error) => {
                    step.status = SimulationStatus::Error;
                    step.port = Some("error".into());
                    step.message = error;
                }
                Ok(input) => {
                    step.input = input;
                    let op = descriptor(&node.config, registry).expect("validated operation");
                    let mismatch = op.input_schema.iter().find(|(key, kind)| {
                        step.input
                            .get(*key)
                            .is_some_and(|v| !type_matches(v, **kind))
                    });
                    let missing = op.required_inputs.iter().find(|key| {
                        step.input.get(*key).is_none_or(|value| {
                            value.is_null()
                                || value.as_str().is_some_and(|text| text.trim().is_empty())
                        })
                    });
                    if let Some(key) = missing {
                        step.status = SimulationStatus::Error;
                        step.port = Some("error".into());
                        step.message = format!("Resolved required input {key} is missing");
                    } else if let Some((key, _)) = mismatch {
                        step.status = SimulationStatus::Error;
                        step.port = Some("error".into());
                        step.message = format!("Resolved input {key} has the wrong type");
                    } else if let Some(output) = fixture.outputs.get(&node.id) {
                        step.output = output.clone();
                        step.message = "Provided fixture; effects suppressed".into();
                        if let Some(port) = output.get("$port").and_then(Value::as_str) {
                            step.port = Some(port.into());
                        }
                    } else if matches!(node.config, NodeConfig::Action { .. }) {
                        step.output = json!({"suppressed": true});
                        step.message = "Action planned; effect suppressed".into();
                    } else {
                        let uncertain = matches!(node.config, NodeConfig::Jev { .. });
                        step.status = if uncertain {
                            SimulationStatus::Uncertain
                        } else {
                            SimulationStatus::Error
                        };
                        step.port = Some(if uncertain { "uncertain" } else { "error" }.into());
                        step.message = "No fixture supplied".into();
                    }
                }
            },
            NodeConfig::Agent {
                prompt,
                output_schema,
                ..
            } => {
                step.input = Value::String(prompt.clone());
                if let Some(output) = fixture.outputs.get(&node.id) {
                    step.output = output.clone();
                    step.message = "Provided agent fixture; no agent invoked".into();
                    if let Err(error) = validate_structured_output(output_schema, output) {
                        step.status = SimulationStatus::Error;
                        step.port = Some("error".into());
                        step.message = error;
                    }
                } else {
                    step.status = SimulationStatus::Uncertain;
                    step.port = Some("uncertain".into());
                    step.message = "No agent fixture supplied; no agent invoked".into();
                }
            }
            NodeConfig::Approval { message } => {
                step.output = json!({"approvalRequired":true});
                step.message = format!("Would pause for approval: {message}");
            }
            NodeConfig::Wait { seconds } => {
                step.output = json!({"seconds":seconds});
                step.message = "Wait planned; time not advanced".into();
            }
        }
        if let Some(port) = &step.port {
            if !ports(&node.config, registry).contains(port) {
                step.status = SimulationStatus::Error;
                step.message = format!("Fixture selected unsupported port {port}");
                step.port = None;
            }
        }
        if let Some(port) = &step.port {
            selected.insert(node.id.clone(), port.clone());
        }
        result.outputs.insert(node.id.clone(), step.output.clone());
        result.steps.push(step);
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Queued,
    Running,
    Waiting,
    AwaitingApproval,
    Paused,
    Succeeded,
    Failed,
    Cancelled,
}
impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunHistory {
    pub sequence: u64,
    pub at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    pub event_key: String,
    pub approved_at: u64,
    pub actor: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    /// Frozen definition is the entire authorization boundary for this run.
    pub workflow: Workflow,
    pub status: RunStatus,
    pub event: Value,
    pub event_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_key: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub history: Vec<RunHistory>,
    pub approvals: Vec<Approval>,
    pub outputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub completed_ports: BTreeMap<String, String>,
    #[serde(default)]
    pub measured_cost_usd: f64,
    #[serde(default)]
    pub cost_unknown: bool,
}
/// Read model derived from the frozen execution graph, never persisted approval hints.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    #[serde(flatten)]
    pub run: Run,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_approval_node_id: Option<String>,
}

impl Run {
    pub fn read_model(&self, registry: &[OperationDescriptor]) -> RunView {
        let pending_approval_node_id = (self.status == RunStatus::AwaitingApproval)
            .then(|| next_ready(self, registry).ok().flatten())
            .flatten()
            .filter(|node| matches!(node.config, NodeConfig::Approval { .. }))
            .filter(|node| {
                !self
                    .approvals
                    .iter()
                    .any(|approval| approval.node_id == node.id)
            })
            .map(|node| node.id);
        RunView {
            run: self.clone(),
            pending_approval_node_id,
        }
    }

    fn record(&mut self, now: u64, node_id: Option<String>, kind: &str, message: String) {
        self.updated_at = now;
        self.history.push(RunHistory {
            sequence: self.history.last().map_or(1, |h| h.sequence + 1),
            at: now,
            node_id,
            kind: kind.into(),
            message,
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationState {
    pub schema_version: u32,
    pub workflows: Vec<Workflow>,
    #[serde(default)]
    pub revisions: Vec<Workflow>,
    pub runs: Vec<Run>,
    pub dedup: BTreeMap<String, String>,
    pub locks: BTreeMap<String, String>,
}
impl Default for AutomationState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            workflows: Vec::new(),
            revisions: Vec::new(),
            runs: Vec::new(),
            dedup: BTreeMap::new(),
            locks: BTreeMap::new(),
        }
    }
}

impl AutomationState {
    pub fn save_workflow(
        &mut self,
        mut workflow: Workflow,
        expected_revision: Option<u64>,
        registry: &[OperationDescriptor],
    ) -> Result<Workflow, String> {
        let issues = validate_workflow(&workflow, registry);
        if !issues.is_empty() {
            return Err(format!(
                "invalid_workflow: {}",
                issues
                    .iter()
                    .map(|i| i.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        if let Some(index) = self.workflows.iter().position(|w| w.id == workflow.id) {
            let existing = &self.workflows[index];
            if expected_revision != Some(existing.revision) {
                return Err("revision_conflict".into());
            }
            workflow.revision = existing
                .revision
                .checked_add(1)
                .ok_or("revision_overflow")?;
            self.revisions.push(existing.clone());
            self.workflows[index] = workflow.clone();
        } else {
            if expected_revision.is_some_and(|r| r != 0) {
                return Err("revision_conflict".into());
            }
            if self.revisions.iter().any(|w| w.id == workflow.id) {
                return Err("workflow_id_retired".into());
            }
            workflow.revision = 1;
            self.workflows.push(workflow.clone());
        }
        Ok(workflow)
    }

    pub fn delete_workflow(&mut self, id: &str) -> Result<(), String> {
        if self
            .runs
            .iter()
            .any(|r| r.workflow.id == id && !r.status.is_terminal())
        {
            return Err("workflow_has_active_runs".into());
        }
        let index = self
            .workflows
            .iter()
            .position(|w| w.id == id)
            .ok_or("workflow_not_found")?;
        self.revisions.push(self.workflows.remove(index));
        Ok(())
    }

    /// Deduplication is scoped to workflow identity and retained across revisions.
    pub fn enqueue(
        &mut self,
        workflow_id: &str,
        event: Value,
        event_key: &str,
        resource_key: Option<String>,
        now: u64,
    ) -> Result<Run, String> {
        if event_key.trim().is_empty() {
            return Err("event_key_required".into());
        }
        let key = serde_json::to_string(&(workflow_id, event_key)).expect("string tuple");
        if let Some(id) = self.dedup.get(&key) {
            return self
                .runs
                .iter()
                .find(|r| &r.id == id)
                .cloned()
                .ok_or_else(|| "dedup_run_missing".into());
        }
        let workflow = self
            .workflows
            .iter()
            .find(|w| w.id == workflow_id)
            .ok_or("workflow_not_found")?;
        if !workflow.enabled {
            return Err("workflow_disabled".into());
        }
        if resource_key
            .as_ref()
            .is_some_and(|key| key.trim().is_empty())
        {
            return Err("resource_key_required".into());
        }
        let mut run = Run {
            id: uuid::Uuid::new_v4().to_string(),
            workflow: workflow.clone(),
            status: RunStatus::Queued,
            event,
            event_key: event_key.into(),
            resource_key,
            created_at: now,
            updated_at: now,
            history: Vec::new(),
            approvals: Vec::new(),
            outputs: BTreeMap::new(),
            completed_ports: BTreeMap::new(),
            measured_cost_usd: 0.0,
            cost_unknown: false,
        };
        run.record(
            now,
            None,
            "queued",
            "Run admitted with a frozen workflow revision".into(),
        );
        self.dedup.insert(key, run.id.clone());
        self.runs.push(run.clone());
        Ok(run)
    }

    /// Only the host may call this with measured adapter metadata. Node outputs and
    /// model-generated text never feed accounting. Unknown cost remains unknown.
    pub fn record_cost(
        &mut self,
        run_id: &str,
        cost: Option<f64>,
        now: u64,
    ) -> Result<Run, String> {
        let run = self
            .runs
            .iter_mut()
            .find(|run| run.id == run_id)
            .ok_or("run_not_found")?;
        if run.status.is_terminal() {
            return Err("run_terminal".into());
        }
        if let Some(cost) = cost {
            if !cost.is_finite()
                || cost < 0.0
                || !run.measured_cost_usd.is_finite()
                || run.measured_cost_usd < 0.0
            {
                return Err("invalid_measured_cost".into());
            }
            let total = run.measured_cost_usd + cost;
            if !total.is_finite() {
                return Err("measured_cost_overflow".into());
            }
            run.measured_cost_usd = total;
            run.record(
                now,
                None,
                "cost_measured",
                format!("Host measured USD {cost}; accumulated USD {total}"),
            );
        } else {
            run.cost_unknown = true;
            run.record(
                now,
                None,
                "cost_unknown",
                "Adapter did not report measurable cost".into(),
            );
        }
        Ok(run.clone())
    }

    pub fn transition(&mut self, run_id: &str, status: RunStatus, now: u64) -> Result<Run, String> {
        let index = self
            .runs
            .iter()
            .position(|r| r.id == run_id)
            .ok_or("run_not_found")?;
        let run = &self.runs[index];
        if run.status == status {
            return Ok(run.clone());
        }
        if run.status.is_terminal() {
            return Err("run_terminal".into());
        }
        let allowed = match run.status {
            RunStatus::Queued => matches!(
                status,
                RunStatus::Running | RunStatus::Paused | RunStatus::Cancelled | RunStatus::Failed
            ),
            RunStatus::Running => !matches!(status, RunStatus::Queued),
            RunStatus::Waiting | RunStatus::AwaitingApproval | RunStatus::Paused => matches!(
                status,
                RunStatus::Running | RunStatus::Paused | RunStatus::Cancelled | RunStatus::Failed
            ),
            _ => false,
        };
        if !allowed {
            return Err("invalid_run_transition".into());
        }
        if status == RunStatus::Succeeded {
            if ready_node(run)?.is_some() {
                return Err("run_has_pending_nodes".into());
            }
            if run.completed_ports.iter().any(|(id, port)| {
                ["error", "uncertain"].contains(&port.as_str())
                    && !run
                        .workflow
                        .edges
                        .iter()
                        .any(|edge| edge.from == *id && edge.port == *port)
            }) {
                return Err("run_has_unhandled_failure".into());
            }
        }
        if status == RunStatus::Running {
            let active = self
                .runs
                .iter()
                .filter(|other| {
                    other.id != run.id
                        && other.workflow.id == run.workflow.id
                        && !other.status.is_terminal()
                        && other.status != RunStatus::Queued
                        && (other.status != RunStatus::Paused
                            || self.locks.values().any(|owner| owner == &other.id))
                })
                .count();
            if active >= run.workflow.policy.max_concurrent_runs as usize {
                return Err("concurrency_limit".into());
            }
            if let Some(resource) = &run.resource_key {
                if self
                    .locks
                    .get(resource)
                    .is_some_and(|owner| owner != run_id)
                {
                    return Err("resource_locked".into());
                }
                self.locks.insert(resource.clone(), run_id.into());
            }
        }
        let run = &mut self.runs[index];
        run.status = status;
        run.record(
            now,
            None,
            "status",
            format!("Run state changed to {status:?}"),
        );
        if status.is_terminal() {
            self.locks.retain(|_, owner| owner != run_id);
        }
        Ok(run.clone())
    }

    pub fn complete_node(
        &mut self,
        run_id: &str,
        node_id: &str,
        port: &str,
        output: Value,
        registry: &[OperationDescriptor],
        now: u64,
    ) -> Result<Run, String> {
        let run = self
            .runs
            .iter_mut()
            .find(|r| r.id == run_id)
            .ok_or("run_not_found")?;
        if run.status != RunStatus::Running {
            return Err("run_not_running".into());
        }
        let next = next_ready(run, registry)?.ok_or("no_ready_node")?;
        if next.id != node_id {
            return Err("node_not_ready".into());
        }
        if !ports(&next.config, registry).iter().any(|p| p == port) {
            return Err("invalid_output_port".into());
        }
        if let NodeConfig::Agent { output_schema, .. } = &next.config {
            if port == "next" {
                validate_structured_output(output_schema, &output)?;
            }
        }
        if matches!(next.config, NodeConfig::Approval { .. })
            && !run
                .approvals
                .iter()
                .any(|a| a.node_id == node_id && a.event_key == run.event_key)
        {
            return Err("approval_required".into());
        }
        run.outputs.insert(node_id.into(), output);
        run.completed_ports.insert(node_id.into(), port.into());
        run.record(
            now,
            Some(node_id.into()),
            "node_completed",
            format!("Selected output {port}"),
        );
        Ok(run.clone())
    }

    /// Recording an observation does not complete a node or advance routing.
    pub fn record_output(
        &mut self,
        run_id: &str,
        node_id: &str,
        output: Value,
        now: u64,
    ) -> Result<Run, String> {
        let run = self
            .runs
            .iter_mut()
            .find(|r| r.id == run_id)
            .ok_or("run_not_found")?;
        if run.status.is_terminal() {
            return Err("run_terminal".into());
        }
        if !run.workflow.nodes.iter().any(|n| n.id == node_id) {
            return Err("node_not_found".into());
        }
        if run.completed_ports.contains_key(node_id) {
            return Err("node_already_completed".into());
        }
        run.outputs.insert(node_id.into(), output);
        run.record(
            now,
            Some(node_id.into()),
            "output",
            "Observation recorded".into(),
        );
        Ok(run.clone())
    }

    pub fn approve(
        &mut self,
        run_id: &str,
        node_id: &str,
        head_sha: Option<String>,
        actor: &str,
        now: u64,
    ) -> Result<Run, String> {
        let run = self
            .runs
            .iter_mut()
            .find(|r| r.id == run_id)
            .ok_or("run_not_found")?;
        if run.status != RunStatus::AwaitingApproval {
            return Err("run_not_awaiting_approval".into());
        }
        if ready_node(run)?.as_ref().map(|node| node.id.as_str()) != Some(node_id) {
            return Err("node_not_ready".into());
        }
        if actor.trim().is_empty() || head_sha.as_ref().is_some_and(|sha| sha.trim().is_empty()) {
            return Err("approval_identity_required".into());
        }
        if !run
            .workflow
            .nodes
            .iter()
            .any(|n| n.id == node_id && matches!(n.config, NodeConfig::Approval { .. }))
        {
            return Err("approval_node_required".into());
        }
        if let Some(sha) = &head_sha {
            if run.event.get("headSha").and_then(Value::as_str) != Some(sha) {
                return Err("approval_sha_mismatch".into());
            }
        }
        if run.approvals.iter().any(|a| a.node_id == node_id) {
            return Err("approval_already_recorded".into());
        }
        run.approvals.push(Approval {
            node_id: node_id.into(),
            head_sha,
            event_key: run.event_key.clone(),
            approved_at: now,
            actor: actor.into(),
        });
        run.record(
            now,
            Some(node_id.into()),
            "approved",
            "Approval bound to the frozen event and optional head SHA".into(),
        );
        Ok(run.clone())
    }

    /// Restart recovery never silently retries effects whose completion is unknown.
    pub fn recover_interrupted(&mut self, now: u64) -> usize {
        let mut count = 0;
        for run in &mut self.runs {
            if run.status == RunStatus::Running {
                run.status = RunStatus::Paused;
                run.record(
                    now,
                    None,
                    "interrupted",
                    "Execution interrupted; explicit review is required before resuming".into(),
                );
                count += 1;
            }
        }
        count
    }
}

/// Returns the frozen run budget still available for the next paid request.
/// This is a preflight bound, not a cost estimate; hosts pass it to supported
/// provider limits and record actual outer response metadata afterward.
pub fn remaining_budget(run: &Run) -> Result<Option<f64>, String> {
    let Some(limit) = run.workflow.policy.max_cost_usd else {
        return Ok(None);
    };
    if !limit.is_finite() || limit <= 0.0 {
        return Err("invalid_budget".into());
    }
    if run.cost_unknown {
        return Err("cost_unknown".into());
    }
    if !run.measured_cost_usd.is_finite() || run.measured_cost_usd < 0.0 {
        return Err("invalid_measured_cost".into());
    }
    let remaining = limit - run.measured_cost_usd;
    if remaining <= 0.0 {
        return Err("budget_exhausted".into());
    }
    Ok(Some(remaining))
}

/// Nodes run after every predecessor settles. Any selected incoming edge enables
/// a join; descendants of unselected branches are skipped.
pub fn next_ready(
    run: &Run,
    registry: &[OperationDescriptor],
) -> Result<Option<WorkflowNode>, String> {
    if !validate_workflow(&run.workflow, registry).is_empty() {
        return Err("invalid_workflow".into());
    }
    ready_node(run)
}

fn ready_node(run: &Run) -> Result<Option<WorkflowNode>, String> {
    let mut skipped = BTreeSet::new();
    for node in topological_order(&run.workflow).ok_or("workflow_cycle")? {
        if run.completed_ports.contains_key(&node.id) {
            continue;
        }
        let incoming: Vec<_> = run
            .workflow
            .edges
            .iter()
            .filter(|edge| edge.to == node.id)
            .collect();
        if incoming.iter().any(|edge| {
            !run.completed_ports.contains_key(&edge.from) && !skipped.contains(&edge.from)
        }) {
            continue;
        }
        if incoming.is_empty()
            || incoming
                .iter()
                .any(|edge| run.completed_ports.get(&edge.from) == Some(&edge.port))
        {
            return Ok(Some(node.clone()));
        }
        skipped.insert(&node.id);
    }
    Ok(None)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MergeChecks {
    pub is_draft: bool,
    pub conflicts: bool,
    pub required_checks_passed: bool,
    pub review_approved: bool,
    pub policy_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MergeContext {
    pub project_id: String,
    pub repository: String,
    pub identity: String,
    pub resource_key: String,
    pub head_sha: String,
    pub checks: MergeChecks,
}

/// Hosts must obtain fresh checks and use the same SHA in their conditional merge
/// request. Passing this gate is not a substitute for remote compare-and-swap.
pub fn merge_gate(
    run: &Run,
    node_id: &str,
    registry: &[OperationDescriptor],
    context: &MergeContext,
) -> Result<(), String> {
    if run.status != RunStatus::Running {
        return Err("run_not_running".into());
    }
    let node = run
        .workflow
        .nodes
        .iter()
        .find(|n| n.id == node_id)
        .ok_or("node_not_found")?;
    if !matches!(node.config, NodeConfig::Action { .. })
        || descriptor(&node.config, registry).is_none_or(|op| op.effect != Effect::Merge)
    {
        return Err("merge_operation_required".into());
    }
    if next_ready(run, registry)?.as_ref().map(|n| n.id.as_str()) != Some(node_id) {
        return Err("node_not_ready".into());
    }
    if !run.workflow.policy.allow_writes {
        return Err("writes_not_authorized".into());
    }
    let scope = &run.workflow.scope;
    if context.project_id.trim().is_empty()
        || context.repository.trim().is_empty()
        || context.identity.trim().is_empty()
        || context.resource_key.trim().is_empty()
        || context.head_sha.trim().is_empty()
    {
        return Err("merge_scope_required".into());
    }
    let target_matches = if scope.targets.is_empty() {
        scope.project_id.as_deref() == Some(context.project_id.as_str())
            && scope.repository.as_deref() == Some(context.repository.as_str())
            && scope.identity.as_deref() == Some(context.identity.as_str())
    } else {
        scope.targets.iter().any(|target| {
            target.project_id == context.project_id
                && target.repository.as_deref() == Some(context.repository.as_str())
                && target.identity.as_deref() == Some(context.identity.as_str())
        }) && run
            .event
            .get("target")
            .and_then(|target| target.get("projectId"))
            .and_then(Value::as_str)
            == Some(context.project_id.as_str())
    };
    if !target_matches || run.resource_key.as_deref() != Some(context.resource_key.as_str()) {
        return Err("merge_scope_mismatch".into());
    }
    if run.event.get("headSha").and_then(Value::as_str) != Some(context.head_sha.as_str()) {
        return Err("merge_sha_changed".into());
    }
    if context.checks.is_draft
        || context.checks.conflicts
        || !context.checks.required_checks_passed
        || !context.checks.review_approved
        || !context.checks.policy_allowed
    {
        return Err("merge_checks_failed".into());
    }
    if run.workflow.policy.require_merge_approval {
        let mut ancestors = BTreeSet::from([node_id]);
        loop {
            let before = ancestors.len();
            for edge in &run.workflow.edges {
                if ancestors.contains(edge.to.as_str()) {
                    ancestors.insert(edge.from.as_str());
                }
            }
            if ancestors.len() == before {
                break;
            }
        }
        if !run.approvals.iter().any(|approval| {
            ancestors.contains(approval.node_id.as_str())
                && run.completed_ports.contains_key(&approval.node_id)
                && approval.head_sha.as_deref() == Some(context.head_sha.as_str())
                && approval.event_key == run.event_key
        }) {
            return Err("merge_approval_required".into());
        }
    }
    Ok(())
}

/// Small explicit schema subset for structured local-agent results. Unsupported
/// constraints are rejected rather than silently treated as authorization.
pub fn validate_output_schema(schema: &Value) -> Result<(), String> {
    let object = schema.as_object().ok_or("output_schema_must_be_object")?;
    for key in object.keys() {
        if ![
            "type",
            "properties",
            "required",
            "enum",
            "items",
            "additionalProperties",
            "description",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unsupported_schema_keyword:{key}"));
        }
    }
    if let Some(kind) = schema.get("type") {
        if ![
            "string", "number", "integer", "boolean", "object", "array", "null",
        ]
        .contains(&kind.as_str().ok_or("schema_type_must_be_string")?)
        {
            return Err("unsupported_schema_type".into());
        }
    }
    if let Some(required) = schema.get("required") {
        for key in required.as_array().ok_or("schema_required_must_be_array")? {
            if !key.is_string() {
                return Err("schema_required_key_must_be_string".into());
            }
        }
    }
    if let Some(options) = schema.get("enum") {
        if !options.is_array() {
            return Err("schema_enum_must_be_array".into());
        }
    }
    if let Some(additional) = schema.get("additionalProperties") {
        if !additional.is_boolean() {
            return Err("schema_additional_properties_must_be_boolean".into());
        }
    }
    if let Some(properties) = schema.get("properties") {
        for property in properties
            .as_object()
            .ok_or("schema_properties_must_be_object")?
            .values()
        {
            validate_output_schema(property)?;
        }
    }
    if let Some(items) = schema.get("items") {
        validate_output_schema(items)?;
    }
    Ok(())
}

pub fn validate_structured_output(schema: &Value, output: &Value) -> Result<(), String> {
    validate_output_schema(schema)?;
    let object = schema.as_object().ok_or("output_schema_must_be_object")?;
    for key in object.keys() {
        if ![
            "type",
            "properties",
            "required",
            "enum",
            "items",
            "additionalProperties",
            "description",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unsupported_schema_keyword:{key}"));
        }
    }
    if let Some(kind) = schema.get("type") {
        let valid = match kind.as_str() {
            Some("string") => output.is_string(),
            Some("number") => output.is_number(),
            Some("integer") => output.is_i64() || output.is_u64(),
            Some("boolean") => output.is_boolean(),
            Some("object") => output.is_object(),
            Some("array") => output.is_array(),
            Some("null") => output.is_null(),
            _ => return Err("unsupported_schema_type".into()),
        };
        if !valid {
            return Err("output_type_mismatch".into());
        }
    }
    if let Some(options) = schema.get("enum") {
        if !options
            .as_array()
            .ok_or("schema_enum_must_be_array")?
            .contains(output)
        {
            return Err("output_enum_mismatch".into());
        }
    }
    if let Some(required) = schema.get("required") {
        for key in required.as_array().ok_or("schema_required_must_be_array")? {
            let key = key.as_str().ok_or("schema_required_key_must_be_string")?;
            if output.get(key).is_none() {
                return Err(format!("output_missing_field:{key}"));
            }
        }
    }
    if let Some(properties) = schema.get("properties") {
        let properties = properties
            .as_object()
            .ok_or("schema_properties_must_be_object")?;
        for (key, property) in properties {
            if let Some(value) = output.get(key) {
                validate_structured_output(property, value)?;
            }
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false))
            && output
                .as_object()
                .is_some_and(|values| values.keys().any(|key| !properties.contains_key(key)))
        {
            return Err("output_unknown_field".into());
        }
    } else if schema.get("additionalProperties") == Some(&Value::Bool(false))
        && output.as_object().is_some_and(|values| !values.is_empty())
    {
        return Err("output_unknown_field".into());
    }
    if let Some(items) = schema.get("items") {
        if let Some(values) = output.as_array() {
            for value in values {
                validate_structured_output(items, value)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, config: NodeConfig) -> WorkflowNode {
        WorkflowNode {
            id: id.into(),
            label: id.into(),
            position: Position::default(),
            config,
        }
    }
    fn edge(from: &str, to: &str, port: &str) -> WorkflowEdge {
        WorkflowEdge {
            from: from.into(),
            to: to.into(),
            port: port.into(),
        }
    }
    fn registry() -> Vec<OperationDescriptor> {
        vec![
            OperationDescriptor {
                id: "query".into(),
                version: 1,
                kind: OperationKind::Query,
                title: "Query".into(),
                required_inputs: vec!["repository".into()],
                input_schema: BTreeMap::from([("repository".into(), InputType::String)]),
                output_ports: vec!["next".into(), "error".into()],
                effect: Effect::Read,
            },
            OperationDescriptor {
                id: "merge".into(),
                version: 1,
                kind: OperationKind::Action,
                title: "Merge".into(),
                output_ports: vec!["next".into(), "error".into()],
                effect: Effect::Merge,
                ..Default::default()
            },
            OperationDescriptor {
                id: "classify".into(),
                version: 1,
                kind: OperationKind::Jev,
                title: "Classify".into(),
                output_ports: vec!["next".into(), "uncertain".into(), "error".into()],
                ..Default::default()
            },
        ]
    }
    fn workflow() -> Workflow {
        Workflow {
            id: "workflow".into(),
            name: "Review work".into(),
            revision: 0,
            enabled: true,
            nodes: vec![node(
                "trigger",
                NodeConfig::Trigger {
                    event: "manual".into(),
                    interval_seconds: None,
                    baseline: Baseline::IgnoreExisting,
                },
            )],
            edges: vec![],
            policy: WorkflowPolicy::default(),
            scope: WorkflowScope::default(),
        }
    }
    fn branching() -> Workflow {
        let mut w = workflow();
        w.nodes.extend([
            node(
                "query",
                NodeConfig::Query {
                    operation: "query".into(),
                    version: 1,
                    inputs: json!({"repository":{"$ref":"event.repository"}}),
                },
            ),
            node(
                "condition",
                NodeConfig::Condition {
                    path: "nodes.query.ready".into(),
                    operator: ConditionOperator::Equals,
                    value: Some(json!(true)),
                },
            ),
            node("yes", NodeConfig::Wait { seconds: 1 }),
            node("no", NodeConfig::Wait { seconds: 2 }),
            node("join", NodeConfig::Wait { seconds: 3 }),
        ]);
        w.edges = vec![
            edge("trigger", "query", "next"),
            edge("query", "condition", "next"),
            edge("condition", "yes", "true"),
            edge("condition", "no", "false"),
            edge("yes", "join", "next"),
            edge("no", "join", "next"),
        ];
        w
    }
    fn state_with(w: Workflow) -> (AutomationState, Run) {
        let mut state = AutomationState::default();
        state.save_workflow(w, None, &registry()).unwrap();
        let run = state
            .enqueue(
                "workflow",
                json!({"headSha":"abc","repository":"org/repo"}),
                "event-1",
                Some("org/repo#1".into()),
                1,
            )
            .unwrap();
        (state, run)
    }
    fn codes(w: &Workflow) -> Vec<String> {
        validate_workflow(w, &registry())
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    #[test]
    fn rejects_cycles_dangling_edges_unknown_ports_and_unreachable_nodes() {
        let mut w = branching();
        assert!(codes(&w).is_empty());
        w.edges.push(edge("join", "query", "next"));
        assert!(codes(&w).contains(&"cycle".into()));
        w.edges.pop();
        w.edges.push(edge("missing", "join", "next"));
        assert!(codes(&w).contains(&"dangling_edge".into()));
        w.edges.pop();
        w.edges[0].port = "success".into();
        assert!(codes(&w).contains(&"port".into()));
        w.nodes
            .push(node("orphan", NodeConfig::Wait { seconds: 1 }));
        assert!(codes(&w).contains(&"unreachable".into()));
    }
    #[test]
    fn validates_version_kind_required_inputs_and_types() {
        let mut w = branching();
        w.nodes[1].config = NodeConfig::Query {
            operation: "query".into(),
            version: 2,
            inputs: json!({}),
        };
        assert!(codes(&w).contains(&"operation".into()));
        w.nodes[1].config = NodeConfig::Action {
            operation: "query".into(),
            version: 1,
            inputs: json!({}),
        };
        assert!(codes(&w).contains(&"operation".into()));
        w.nodes[1].config = NodeConfig::Query {
            operation: "query".into(),
            version: 1,
            inputs: json!({}),
        };
        assert!(codes(&w).contains(&"required_input".into()));
        w.nodes[1].config = NodeConfig::Query {
            operation: "query".into(),
            version: 1,
            inputs: json!({"repository":3}),
        };
        assert!(codes(&w).contains(&"input_type".into()));
    }
    #[test]
    fn simulation_walks_selected_branch_and_join_in_dependency_order() {
        let w = branching();
        let result = simulate(
            &w,
            &registry(),
            &SimulationFixture {
                event: json!({"repository":"org/repo"}),
                outputs: BTreeMap::from([("query".into(), json!({"ready":false}))]),
            },
        )
        .unwrap();
        assert!(result.effects_suppressed);
        assert_eq!(
            result
                .steps
                .iter()
                .find(|s| s.node_id == "yes")
                .unwrap()
                .status,
            SimulationStatus::Skipped
        );
        assert_eq!(
            result
                .steps
                .iter()
                .find(|s| s.node_id == "no")
                .unwrap()
                .status,
            SimulationStatus::Simulated
        );
        assert_eq!(
            result
                .steps
                .iter()
                .find(|s| s.node_id == "join")
                .unwrap()
                .status,
            SimulationStatus::Simulated
        );
        assert_eq!(
            result
                .steps
                .iter()
                .find(|s| s.node_id == "query")
                .unwrap()
                .input,
            json!({"repository":"org/repo"})
        );
    }
    #[test]
    fn missing_query_observation_does_not_fabricate_condition_success() {
        let result = simulate(
            &branching(),
            &registry(),
            &SimulationFixture {
                event: json!({"repository":"org/repo"}),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.steps[1].status, SimulationStatus::Error);
        assert!(result
            .steps
            .iter()
            .filter(|s| ["yes", "no", "join"].contains(&s.node_id.as_str()))
            .all(|s| s.status == SimulationStatus::Skipped));
    }
    #[test]
    fn missing_condition_field_takes_error_not_false_or_not_equals() {
        let mut w = workflow();
        w.nodes.push(node(
            "condition",
            NodeConfig::Condition {
                path: "event.missing".into(),
                operator: ConditionOperator::NotEquals,
                value: Some(json!(true)),
            },
        ));
        w.edges.push(edge("trigger", "condition", "next"));
        let result = simulate(&w, &registry(), &SimulationFixture::default()).unwrap();
        assert_eq!(result.steps[1].port.as_deref(), Some("error"));
    }
    #[test]
    fn missing_judgment_is_uncertain_and_effects_are_suppressed() {
        let mut w = workflow();
        w.nodes.push(node(
            "judge",
            NodeConfig::Jev {
                operation: "classify".into(),
                version: 1,
                inputs: json!({}),
            },
        ));
        w.edges.push(edge("trigger", "judge", "next"));
        let result = simulate(&w, &registry(), &SimulationFixture::default()).unwrap();
        assert_eq!(result.steps[1].status, SimulationStatus::Uncertain);
        w.nodes[1].config = NodeConfig::Action {
            operation: "merge".into(),
            version: 1,
            inputs: json!({}),
        };
        let result = simulate(&w, &registry(), &SimulationFixture::default()).unwrap();
        assert_eq!(result.steps[1].output, json!({"suppressed":true}));
    }
    #[test]
    fn resolved_reference_must_satisfy_operation_type() {
        let result = simulate(
            &branching(),
            &registry(),
            &SimulationFixture {
                event: json!({"repository":123}),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.steps[1].status, SimulationStatus::Error);
    }
    #[test]
    fn revisions_use_compare_and_swap_and_preserve_run_snapshot() {
        let (mut state, run) = state_with(workflow());
        let mut edited = state.workflows[0].clone();
        edited.name = "Changed".into();
        edited.policy.allow_writes = true;
        assert_eq!(
            state
                .save_workflow(edited.clone(), Some(0), &registry())
                .unwrap_err(),
            "revision_conflict"
        );
        assert_eq!(
            state
                .save_workflow(edited, Some(1), &registry())
                .unwrap()
                .revision,
            2
        );
        assert_eq!(state.revisions[0].revision, 1);
        assert_eq!(state.runs[0].workflow, run.workflow);
        assert!(!state.runs[0].workflow.policy.allow_writes);
    }
    #[test]
    fn duplicate_events_survive_serde_round_trip_and_revision_edits() {
        let (state, run) = state_with(workflow());
        let mut state: AutomationState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        let mut changed = state.workflows[0].clone();
        changed.name = "New revision".into();
        state.save_workflow(changed, Some(1), &registry()).unwrap();
        let duplicate = state
            .enqueue("workflow", json!({}), "event-1", None, 2)
            .unwrap();
        assert_eq!(duplicate.id, run.id);
        assert_eq!(state.runs.len(), 1);
    }
    #[test]
    fn resource_lock_spans_workflows_waits_and_restart() {
        let (mut state, run) = state_with(workflow());
        state.transition(&run.id, RunStatus::Running, 2).unwrap();
        let mut w = workflow();
        w.id = "other".into();
        state.save_workflow(w, None, &registry()).unwrap();
        let other = state
            .enqueue("other", json!({}), "event-1", Some("org/repo#1".into()), 3)
            .unwrap();
        assert_eq!(
            state
                .transition(&other.id, RunStatus::Running, 4)
                .unwrap_err(),
            "resource_locked"
        );
        assert_eq!(state.recover_interrupted(5), 1);
        assert_eq!(state.runs[0].status, RunStatus::Paused);
        assert_eq!(
            state
                .transition(&other.id, RunStatus::Running, 6)
                .unwrap_err(),
            "resource_locked"
        );
        state.transition(&run.id, RunStatus::Cancelled, 7).unwrap();
        state.transition(&other.id, RunStatus::Running, 8).unwrap();
    }
    #[test]
    fn concurrency_without_resource_is_bounded() {
        let (mut state, run) = state_with(workflow());
        state.transition(&run.id, RunStatus::Running, 2).unwrap();
        let other = state
            .enqueue("workflow", json!({}), "event-2", None, 3)
            .unwrap();
        assert_eq!(
            state
                .transition(&other.id, RunStatus::Running, 4)
                .unwrap_err(),
            "concurrency_limit"
        );
    }
    #[test]
    fn actual_executor_skips_unselected_branch_and_preserves_join() {
        let (mut state, run) = state_with(branching());
        state.transition(&run.id, RunStatus::Running, 2).unwrap();
        for (id, port) in [
            ("trigger", "next"),
            ("query", "next"),
            ("condition", "false"),
            ("no", "next"),
            ("join", "next"),
        ] {
            assert_eq!(
                next_ready(&state.runs[0], &registry()).unwrap().unwrap().id,
                id
            );
            state
                .complete_node(&run.id, id, port, json!({}), &registry(), 3)
                .unwrap();
        }
        assert!(next_ready(&state.runs[0], &registry()).unwrap().is_none());
        state.transition(&run.id, RunStatus::Succeeded, 4).unwrap();
        assert_eq!(
            state
                .transition(&run.id, RunStatus::Running, 5)
                .unwrap_err(),
            "run_terminal"
        );
    }
    #[test]
    fn cannot_complete_unready_node_or_finish_with_pending_nodes() {
        let (mut state, run) = state_with(branching());
        state.transition(&run.id, RunStatus::Running, 2).unwrap();
        assert_eq!(
            state
                .complete_node(&run.id, "query", "next", json!({}), &registry(), 3)
                .unwrap_err(),
            "node_not_ready"
        );
        assert_eq!(
            state
                .transition(&run.id, RunStatus::Succeeded, 4)
                .unwrap_err(),
            "run_has_pending_nodes"
        );
    }
    fn merge_run() -> (AutomationState, Run, MergeContext) {
        let mut w = workflow();
        w.policy.allow_writes = true;
        w.scope = WorkflowScope {
            project_id: Some("project".into()),
            repository: Some("org/repo".into()),
            identity: Some("owner".into()),
            ..Default::default()
        };
        w.nodes.extend([
            node(
                "approval",
                NodeConfig::Approval {
                    message: "Merge exact head".into(),
                },
            ),
            node(
                "merge",
                NodeConfig::Action {
                    operation: "merge".into(),
                    version: 1,
                    inputs: json!({}),
                },
            ),
        ]);
        w.edges = vec![
            edge("trigger", "approval", "next"),
            edge("approval", "merge", "next"),
        ];
        let (mut state, run) = state_with(w);
        state.transition(&run.id, RunStatus::Running, 2).unwrap();
        state
            .complete_node(&run.id, "trigger", "next", json!({}), &registry(), 3)
            .unwrap();
        state
            .transition(&run.id, RunStatus::AwaitingApproval, 4)
            .unwrap();
        let context = MergeContext {
            project_id: "project".into(),
            repository: "org/repo".into(),
            identity: "owner".into(),
            resource_key: "org/repo#1".into(),
            head_sha: "abc".into(),
            checks: MergeChecks {
                required_checks_passed: true,
                review_approved: true,
                policy_allowed: true,
                ..Default::default()
            },
        };
        (state, run, context)
    }
    #[test]
    fn approval_binds_exact_sha_and_cannot_be_rewritten() {
        let (mut state, run, _) = merge_run();
        assert_eq!(
            state
                .approve(&run.id, "approval", Some("changed".into()), "person", 5)
                .unwrap_err(),
            "approval_sha_mismatch"
        );
        state
            .approve(&run.id, "approval", Some("abc".into()), "person", 5)
            .unwrap();
        assert_eq!(
            state
                .approve(&run.id, "approval", Some("abc".into()), "person", 6)
                .unwrap_err(),
            "approval_already_recorded"
        );
    }
    #[test]
    fn merge_requires_frozen_grant_scope_approval_sha_and_all_checks() {
        let (mut state, run, mut context) = merge_run();
        state
            .approve(&run.id, "approval", Some("abc".into()), "person", 5)
            .unwrap();
        state.transition(&run.id, RunStatus::Running, 6).unwrap();
        state
            .complete_node(&run.id, "approval", "next", json!({}), &registry(), 7)
            .unwrap();
        assert!(merge_gate(&state.runs[0], "merge", &registry(), &context).is_ok());
        context.head_sha = "changed".into();
        assert_eq!(
            merge_gate(&state.runs[0], "merge", &registry(), &context).unwrap_err(),
            "merge_sha_changed"
        );
        context.head_sha = "abc".into();
        context.identity = "stranger".into();
        assert_eq!(
            merge_gate(&state.runs[0], "merge", &registry(), &context).unwrap_err(),
            "merge_scope_mismatch"
        );
        context.identity = "owner".into();
        context.checks.required_checks_passed = false;
        assert_eq!(
            merge_gate(&state.runs[0], "merge", &registry(), &context).unwrap_err(),
            "merge_checks_failed"
        );
        context.checks.required_checks_passed = true;
        state.runs[0].approvals.clear();
        assert_eq!(
            merge_gate(&state.runs[0], "merge", &registry(), &context).unwrap_err(),
            "merge_approval_required"
        );
    }
    #[test]
    fn generic_approval_does_not_authorize_merge() {
        let (mut state, run, context) = merge_run();
        state
            .approve(&run.id, "approval", None, "person", 5)
            .unwrap();
        state.transition(&run.id, RunStatus::Running, 6).unwrap();
        state
            .complete_node(&run.id, "approval", "next", json!({}), &registry(), 7)
            .unwrap();
        assert_eq!(
            merge_gate(&state.runs[0], "merge", &registry(), &context).unwrap_err(),
            "merge_approval_required"
        );
    }
    #[test]
    fn structured_output_is_strict_and_unknown_constraints_fail() {
        let schema = json!({"type":"object","properties":{"result":{"type":"string","enum":["done","blocked"]}},"required":["result"],"additionalProperties":false});
        assert!(validate_structured_output(&schema, &json!({"result":"done"})).is_ok());
        for output in [
            json!({}),
            json!({"result":"maybe"}),
            json!({"result":"done","extra":true}),
        ] {
            assert!(validate_structured_output(&schema, &output).is_err());
        }
        assert!(validate_structured_output(&json!({"oneOf":[]}), &json!({})).is_err());
    }
    #[test]
    fn shared_condition_rules_match_simulation_for_null_missing_and_arrays() {
        let event = json!({"nothing":null,"items":[false,true],"text":"true"});
        for (path, operator, expected) in [
            ("event.nothing", ConditionOperator::Exists, None),
            ("event.missing", ConditionOperator::Exists, None),
            (
                "event.missing",
                ConditionOperator::NotEquals,
                Some(json!(true)),
            ),
            ("event.items.1", ConditionOperator::Truthy, None),
            (
                "/event/items/0",
                ConditionOperator::Equals,
                Some(json!(false)),
            ),
            ("event.text", ConditionOperator::Truthy, None),
        ] {
            let mut w = workflow();
            w.nodes.push(node(
                "condition",
                NodeConfig::Condition {
                    path: path.into(),
                    operator,
                    value: expected.clone(),
                },
            ));
            w.edges.push(edge("trigger", "condition", "next"));
            let result = simulate(
                &w,
                &registry(),
                &SimulationFixture {
                    event: event.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
            match evaluate_condition(&json!({"event":event}), path, operator, expected.as_ref()) {
                Ok(value) => {
                    assert_eq!(result.steps[1].output, json!(value));
                    assert_eq!(result.steps[1].port, Some(value.to_string()));
                }
                Err(_) => assert_eq!(result.steps[1].port.as_deref(), Some("error")),
            }
        }
        assert_eq!(
            resolve_value(&json!({"$ref":"event.items.1"}), &json!({"event":event})).unwrap(),
            json!(true)
        );
    }

    #[test]
    fn terminal_unhandled_error_cannot_be_reported_as_success() {
        let (mut state, run) = state_with(workflow());
        state.transition(&run.id, RunStatus::Running, 2).unwrap();
        state
            .complete_node(&run.id, "trigger", "error", json!({}), &registry(), 3)
            .unwrap();
        assert_eq!(
            state
                .transition(&run.id, RunStatus::Succeeded, 4)
                .unwrap_err(),
            "run_has_unhandled_failure"
        );
        state.transition(&run.id, RunStatus::Failed, 4).unwrap();
    }

    #[test]
    fn agent_completion_enforces_declared_output_schema() {
        let mut w = workflow();
        w.nodes.push(node(
            "agent",
            NodeConfig::Agent {
                prompt: "Inspect".into(),
                provider: None,
                context: json!({}),
                tools: vec!["read_file".into()],
                checks: vec![],
                output_schema: json!({"type":"object", "required":["result"]}),
            },
        ));
        w.edges.push(edge("trigger", "agent", "next"));
        let (mut state, run) = state_with(w);
        state.transition(&run.id, RunStatus::Running, 2).unwrap();
        state
            .complete_node(&run.id, "trigger", "next", json!({}), &registry(), 3)
            .unwrap();
        assert!(state
            .complete_node(&run.id, "agent", "next", json!({}), &registry(), 4)
            .is_err());
        assert!(!state.runs[0].completed_ports.contains_key("agent"));
        state
            .complete_node(
                &run.id,
                "agent",
                "next",
                json!({"result":"done"}),
                &registry(),
                5,
            )
            .unwrap();
    }

    #[test]
    fn undeclared_inputs_and_unsupported_agent_constraints_are_rejected() {
        let mut w = branching();
        w.nodes[1].config = NodeConfig::Query {
            operation: "query".into(),
            version: 1,
            inputs: json!({"repository":"org/repo","command":"unexpected"}),
        };
        assert!(codes(&w).contains(&"unknown_input".into()));
        let schema = json!({"type":"object","properties":{"optional":{"pattern":".*"}}});
        assert!(validate_structured_output(&schema, &json!({})).is_err());
    }

    #[test]
    fn merge_multiple_targets_remains_bound_to_one_frozen_event_target() {
        let (mut state, run, context) = merge_run();
        state.runs[0].workflow.scope.targets = vec![WorkflowTarget {
            project_id: "project".into(),
            repository: Some("org/repo".into()),
            identity: Some("owner".into()),
        }];
        state
            .approve(&run.id, "approval", Some("abc".into()), "person", 5)
            .unwrap();
        state.transition(&run.id, RunStatus::Running, 6).unwrap();
        state
            .complete_node(&run.id, "approval", "next", json!({}), &registry(), 7)
            .unwrap();
        assert_eq!(
            merge_gate(&state.runs[0], "merge", &registry(), &context).unwrap_err(),
            "merge_scope_mismatch"
        );
        state.runs[0].event["target"] = json!({"projectId":"project"});
        assert!(merge_gate(&state.runs[0], "merge", &registry(), &context).is_ok());
        state.runs[0].event["target"] = json!({"projectId":"other"});
        assert_eq!(
            merge_gate(&state.runs[0], "merge", &registry(), &context).unwrap_err(),
            "merge_scope_mismatch"
        );
    }

    #[test]
    fn measured_budget_accumulates_and_exhaustion_blocks_next_request() {
        let mut w = workflow();
        w.policy.max_cost_usd = Some(1.0);
        let (mut state, run) = state_with(w);
        assert_eq!(remaining_budget(&run).unwrap(), Some(1.0));
        state.record_cost(&run.id, Some(0.25), 2).unwrap();
        assert_eq!(remaining_budget(&state.runs[0]).unwrap(), Some(0.75));
        state.record_cost(&run.id, Some(0.75), 3).unwrap();
        assert_eq!(
            remaining_budget(&state.runs[0]).unwrap_err(),
            "budget_exhausted"
        );
    }

    #[test]
    fn missing_adapter_cost_never_becomes_zero_or_known_again() {
        let mut w = workflow();
        w.policy.max_cost_usd = Some(1.0);
        let (mut state, run) = state_with(w);
        state.record_cost(&run.id, None, 2).unwrap();
        state.record_cost(&run.id, Some(0.1), 3).unwrap();
        assert!(state.runs[0].cost_unknown);
        assert_eq!(
            remaining_budget(&state.runs[0]).unwrap_err(),
            "cost_unknown"
        );
        state.runs[0].workflow.policy.max_cost_usd = None;
        assert_eq!(remaining_budget(&state.runs[0]).unwrap(), None);
    }

    #[test]
    fn budget_and_measured_values_reject_negative_nonfinite_and_overflow() {
        for cost in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut w = workflow();
            w.policy.max_cost_usd = Some(cost);
            assert!(codes(&w).contains(&"budget".into()));
        }
        let (mut state, run) = state_with(workflow());
        for cost in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(state.record_cost(&run.id, Some(cost), 2).is_err());
        }
        assert_eq!(state.runs[0].measured_cost_usd, 0.0);
        state.record_cost(&run.id, Some(f64::MAX), 3).unwrap();
        assert_eq!(
            state.record_cost(&run.id, Some(f64::MAX), 4).unwrap_err(),
            "measured_cost_overflow"
        );
        assert_eq!(state.runs[0].measured_cost_usd, f64::MAX);
    }

    #[test]
    fn model_cost_claims_do_not_change_accounting_and_budget_is_frozen() {
        let mut w = workflow();
        w.policy.max_cost_usd = Some(2.0);
        let (mut state, run) = state_with(w);
        state
            .record_output(
                &run.id,
                "trigger",
                json!({"costUsd":0.01,"total_cost_usd":0.01}),
                2,
            )
            .unwrap();
        assert_eq!(state.runs[0].measured_cost_usd, 0.0);
        let mut revised = state.workflows[0].clone();
        revised.policy.max_cost_usd = Some(100.0);
        state.save_workflow(revised, Some(1), &registry()).unwrap();
        assert_eq!(remaining_budget(&state.runs[0]).unwrap(), Some(2.0));
        let restored: AutomationState =
            serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(remaining_budget(&restored.runs[0]).unwrap(), Some(2.0));
    }

    #[test]
    fn fixed_agent_checks_require_configuration_and_have_bounded_literal_argv() {
        let mut w = workflow();
        let mut config: NodeConfig = serde_json::from_value(
            json!({"type":"agent","prompt":"Fix and verify","tools":["run_checks"]}),
        )
        .unwrap();
        w.nodes.push(node("agent", config.clone()));
        w.edges.push(edge("trigger", "agent", "next"));
        assert!(codes(&w).contains(&"agent_checks".into()));
        if let NodeConfig::Agent { checks, .. } = &mut config {
            checks.push(CheckCommand {
                executable: "cargo".into(),
                args: vec!["test".into(), "--locked".into()],
            });
        }
        w.nodes[1].config = config;
        assert!(codes(&w).is_empty());
        let command = CheckCommand {
            executable: "cargo".into(),
            args: vec!["test".into()],
        };
        assert!(validate_check_commands(&vec![command.clone(); 8]).is_ok());
        assert!(validate_check_commands(&vec![command.clone(); 9]).is_err());
        for executable in ["sh -c", "/bin/sh", "cargo;touch", "", ".."] {
            assert!(validate_check_commands(&[CheckCommand {
                executable: executable.into(),
                args: vec![]
            }])
            .is_err());
        }
        for argument in ["*", "src/*.rs", "test?", "[abc]", "line\nline", "bad\0arg"] {
            assert!(validate_check_commands(&[CheckCommand {
                executable: "cargo".into(),
                args: vec![argument.into()]
            }])
            .is_err());
        }
        assert!(validate_check_commands(&[CheckCommand {
            executable: "cargo".into(),
            args: vec!["x".into(); 65]
        }])
        .is_err());
        assert!(validate_check_commands(&[CheckCommand {
            executable: "cargo".into(),
            args: vec!["x".repeat(4097)]
        }])
        .is_err());
    }

    #[test]
    fn check_configuration_is_frozen_and_cannot_be_replaced_by_node_output() {
        let mut w = workflow();
        w.nodes.push(node("agent", serde_json::from_value(json!({"type":"agent","prompt":"Verify","tools":["run_checks"],"checks":[{"executable":"cargo","args":["test","--locked"]}]})).unwrap()));
        w.edges.push(edge("trigger", "agent", "next"));
        let (mut state, run) = state_with(w);
        let mut edited = state.workflows[0].clone();
        if let NodeConfig::Agent { checks, .. } = &mut edited.nodes[1].config {
            checks[0].args = vec!["check".into()];
        }
        state.save_workflow(edited, Some(1), &registry()).unwrap();
        state
            .record_output(
                &run.id,
                "agent",
                json!({"checks":[{"executable":"sh","args":["-c","echo changed"]}]}),
                2,
            )
            .unwrap();
        let NodeConfig::Agent { checks, .. } = &state.runs[0].workflow.nodes[1].config else {
            panic!("agent");
        };
        assert_eq!(checks[0].executable, "cargo");
        assert_eq!(checks[0].args, vec!["test", "--locked"]);
        assert_eq!(state.runs[0].workflow, run.workflow);
    }

    #[test]
    fn legacy_policy_never_inherits_commit_or_push_permission_from_writes() {
        let policy: WorkflowPolicy =
            serde_json::from_value(json!({"allowWrites":true,"requireMergeApproval":false}))
                .unwrap();
        assert!(policy.allow_writes);
        assert!(!policy.allow_commit);
        assert!(!policy.allow_push);
        assert!(policy.require_publish_approval);
        let wire = serde_json::to_value(&policy).unwrap();
        assert_eq!(wire["allowCommit"], false);
        assert_eq!(wire["allowPush"], false);
        assert_eq!(wire["requirePublishApproval"], true);
    }

    #[test]
    fn commit_push_and_publish_approval_are_independent_explicit_grants() {
        let mut w = workflow();
        w.policy.allow_commit = true;
        assert!(validate_workflow(&w, &registry()).is_empty());
        assert!(!w.policy.allow_push);
        assert!(w.policy.require_publish_approval);
        // A host may publish an existing human-created commit without creating one.
        w.policy.allow_commit = false;
        w.policy.allow_push = true;
        assert!(validate_workflow(&w, &registry()).is_empty());
        let restored: Workflow = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
        assert_eq!(restored.policy, w.policy);
    }

    #[test]
    fn later_publication_grants_cannot_escalate_an_existing_run() {
        let (mut state, run) = state_with(workflow());
        let mut revised = state.workflows[0].clone();
        revised.policy.allow_writes = true;
        revised.policy.allow_commit = true;
        revised.policy.allow_push = true;
        revised.policy.require_publish_approval = false;
        state.save_workflow(revised, Some(1), &registry()).unwrap();
        let old = &state.runs[0];
        assert_eq!(old.id, run.id);
        assert!(!old.workflow.policy.allow_commit);
        assert!(!old.workflow.policy.allow_push);
        assert!(old.workflow.policy.require_publish_approval);
        let new_run = state
            .enqueue("workflow", json!({}), "event-2", None, 2)
            .unwrap();
        assert!(new_run.workflow.policy.allow_commit);
        assert!(new_run.workflow.policy.allow_push);
        assert!(!new_run.workflow.policy.require_publish_approval);
        let restored: AutomationState =
            serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(restored.runs[0].workflow.policy, run.workflow.policy);
    }

    #[test]
    fn recording_approval_does_not_grant_commit_or_push_authority() {
        let (mut state, run, _) = merge_run();
        state
            .approve(&run.id, "approval", Some("abc".into()), "person", 5)
            .unwrap();
        assert!(!state.runs[0].workflow.policy.allow_commit);
        assert!(!state.runs[0].workflow.policy.allow_push);
        assert!(state.runs[0].workflow.policy.require_publish_approval);
    }

    #[test]
    fn serialization_contract_uses_camel_case_tagged_nodes_and_safe_defaults() {
        let value = serde_json::to_value(workflow()).unwrap();
        assert_eq!(value["nodes"][0]["config"]["type"], "trigger");
        assert_eq!(value["policy"]["allowWrites"], false);
        assert_eq!(value["nodes"][0]["config"]["baseline"], "ignoreExisting");
        let config: NodeConfig =
            serde_json::from_value(json!({"type":"agent","prompt":"Review"})).unwrap();
        assert!(
            matches!(config,NodeConfig::Agent { tools,checks,context,output_schema,.. } if tools.is_empty() && checks.is_empty() && context == json!({}) && output_schema == json!({}))
        );
    }
}
