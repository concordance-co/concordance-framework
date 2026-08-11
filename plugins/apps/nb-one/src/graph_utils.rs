use std::{fs, hash::RandomState, path::Path};

use crate::{
    llm_utils::{generate_inputs, mcp_tools_split, ThreadStorage},
    plugin::injector::{
        error::FsError,
        host::{log, new_client, Level},
        open_a_i_like::{
            ChatCompletion, ChatSession, ContentType, FunctionUsage, Message, MessageContent,
            ToolCallUsage, ToolSelection,
        },
    },
};
// use arrow_schema::{DataType, Field, Schema as ArrowSchema};
use chrono::{DateTime, Utc};
use daggy::{Dag, EdgeIndex, NodeIndex, Walker};
use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use shared::{inlined_schema_for, types::EmbeddingConfig, TryFromEnvVar};
// use uuid::Uuid;

use crate::plugin::injector::{
    env::PluginError,
    host::{call_plugin, connect_db},
    open_a_i_like::Client,
    vector_db::SimilaritySearchConfig,
};

use std::sync::LazyLock;

pub static PATH_SELECTION_SCHEMA: LazyLock<Schema> =
    LazyLock::new(|| inlined_schema_for!(ChoosePath));
pub static PATH_SELECTION_TOOL: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "choose_path",
            "description": "Select an existing path to traverse",
            "parameters": PATH_SELECTION_SCHEMA.as_value()
        }
    })
});

pub static CONNECT_TO_NODE_SCHEMA: LazyLock<Schema> =
    LazyLock::new(|| inlined_schema_for!(ConnectToNode));
pub static CONNECT_TO_NODE_TOOL: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "connect_to_node",
            "description": "Create a new edge, connecting to the target node",
            "parameters": CONNECT_TO_NODE_SCHEMA.as_value()
        }
    })
});

pub static PLAN_PATH_SCHEMA: LazyLock<Schema> = LazyLock::new(|| inlined_schema_for!(PlanPath));
pub static PLAN_PATH_TOOL: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "plan_path",
            "description": "Plan a path and create new nodes in the graph",
            "parameters": PLAN_PATH_SCHEMA.as_value()
        }
    })
});

pub static CREATE_NEW_PATH_SCHEMA: LazyLock<Schema> =
    LazyLock::new(|| inlined_schema_for!(CreateNewPath));
pub static CREATE_NEW_PATH_TOOL: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "create_new_path",
            "description": "Create a new path of nodes",
            "parameters": CREATE_NEW_PATH_SCHEMA.as_value()
        }
    })
});

pub static JUMP_TO_ROOT_TOOL: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "jump_to_root",
            "description": "Jump to the root node. Call this if the you believe the user's intent
            has been fulfilled in previous messages, or if the user has requested an unrelated action.",
            "parameters": {}
        }
    })
});

pub fn default_tools() -> Vec<String> {
    let tools = vec![
        PATH_SELECTION_TOOL.clone(),
        CONNECT_TO_NODE_TOOL.clone(),
        PLAN_PATH_TOOL.clone(),
        JUMP_TO_ROOT_TOOL.clone(),
    ];

    tools.into_iter().map(|t| t.to_string()).collect()
}

/// Every node in the intent-DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub description: String,
    pub root: bool,
    pub mutable: bool,
    pub stoppable: bool,
    pub kind: NodeKind,
    pub weight: f32, // 0 - 100
    pub action: ActionType,
    /// Optional creation metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_via: Option<Author>, // as a result of human feedback or agent reasoning
}

/// Payload varies by node kind
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKind {
    System(SystemNode),
    Agent(AgentNode),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "subkind", rename_all = "snake_case")]
pub enum SystemNode {
    RootIntention {
        text: String,
    }, // text for longer explanatory string
    /// Any immutable sub-directive (e.g. "Build a graph")
    InitDirective {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "subkind", rename_all = "snake_case")]
pub enum AgentNode {
    /// Represents a sub-intent node in the graph
    ///
    /// These nodes represent goals or intentions that the agent
    /// wants to achieve as part of its operation
    SubIntention {
        /// Detailed description of the sub-intention
        text: String,
        /// A numerical value between 0 and 1 representing the perceived
        /// utility or importance of this sub-intention
        utility: f32,
        /// Current execution status (Active, Blocked, Done)
        status: Status,
    },
    /// A tool invocation specification node
    ///
    /// Represents a node that triggers external tool execution with
    /// specified parameters
    ToolCall {
        /// The name/identifier of the tool to be invoked. Must exist in the list of tools provided in the context.
        tool: String,
        /// Whether additional input is needed for the tool call - i.e. if the hardcoded arguments are insufficient
        needs_additional_input_generation: bool,
        /// Hardcoded arguments for the tool call. This must be valid JSON string and a subset or entire set of
        /// the schema. For a given argument, if you want it to be dynamic, have the value be exactly: "<CONC_DYN>" (quotes included)
        hardcoded_args: String,
        #[schemars(skip)]
        #[serde(default = "serde_json::Value::default")]
        filtered_schema: serde_json::Value,
    },
    /// Stored factual knowledge or memory reference node
    ///
    /// Used to maintain context and factual information within the graph
    /// that can be referenced during execution
    Context {
        /// Origin of the information (e.g., "url", "slack", "internal")
        source: String,
        /// Concise representation of the contextual information
        summary: String,
        /// Optional identifier for retrieving the data from a vector store
        vector_id: Option<String>,
    },
}

impl Node {
    pub fn new(
        description: String,
        root: bool,
        kind: NodeKind,
        weight: f32,
        mutable: bool,
        action: ActionType,
    ) -> Self {
        Node {
            description,
            root,
            kind,
            weight,
            mutable,
            stoppable: false,
            action,
            created_at: Some(Utc::now()),
            created_via: Some(Author::Agent),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Edge {
    scent: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GraphTrace {
    /// The index of the associated message in the thread
    pub message_idx: usize,
    /// A vector of node indices that had a high similarity to
    /// the message prompt, representing potential semantic matches
    pub embedding_options: EmbeddingOptions,
    /// Collection of possible graph traversal paths that could be taken
    pub traversal_options: TraversalOptions,
    /// Index of the selected traversal option that was chosen
    pub selected_trace_idx: Option<usize>,
    /// Flag indicating whether user feedback was provided for this trace
    pub feedback: bool,
}

/// Type alias for a collection of node indices representing embedding-based matches
pub type EmbeddingOptions = Vec<NodeIndex>;
/// Type alias for a collection of possible traversal paths through the graph
pub type TraversalOptions = Vec<TraversalOption>;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TraversalOption {
    /// The sequence of edges to follow from the start node, representing a path through the graph
    pub edges: Vec<EdgeIndex>,
}

impl TraversalOption {
    pub fn scent(&self, graph: &Dag<Node, Edge>) -> f64 {
        f64::powf(
            self.edges
                .iter()
                .map(|edge| graph.edge_weight(*edge).unwrap().scent)
                .fold(1., |acc, scent| acc * scent),
            1. / self.edges.len() as f64,
        )
    }

    /// Concatenate the description of each node
    pub fn context(&self, graph: &Dag<Node, Edge>) -> String {
        // Function for adding context to a traversal - Experiment here
        self.edges
            .iter()
            .filter_map(|edge| {
                let (_start, end) = graph.edge_endpoints(*edge)?;
                Some(graph.node_weight(end)?.description.clone())
            })
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    /// Walks the traversal, executing each node's action building context along the way
    pub fn execute(
        &self,
        thread: &mut ThreadStorage,
        graph: &Dag<Node, Edge>,
        chat_session: &mut ChatSession,
    ) -> Result<String, PluginError> {
        let mut context = String::new();
        for edge in &self.edges {
            let (start, end) = graph.edge_endpoints(*edge).unwrap();
            if thread.current_node != start {
                thread.current_node = start;
            }
            graph
                .node_weight(end)
                .unwrap()
                .execute(&mut context, chat_session)?;
            // log(
            //     Level::Info,
            //     &format!(
            //         "Setting current node to {:?} -- currently: {:?} -- max: {:?}",
            //         end,
            //         thread.current_node,
            //         graph.node_count()
            //     ),
            // );
            assert!(
                end.index() < graph.node_count(),
                "end is the node count????"
            );
            thread.current_node = end;
        }
        Ok(context)
    }
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
/// Given a number of different graph traversals to interact with the world, selects one of the
/// existing provided traversal options
pub struct ChoosePath {
    /// The index of the traversal path to take
    pub traversal_option: usize,
}

impl ChoosePath {
    pub fn execute(
        &self,
        usr_msg_idx: usize,
        traversal_options: &TraversalOptions,
        embedding_options: &EmbeddingOptions,
        dag: &Dag<Node, Edge>,
        thread: &mut ThreadStorage,
        chat_session: &mut ChatSession,
    ) -> Result<String, PluginError> {
        // Implementation of the connect_to_node tool
        // Step 1: Get the traversal option from the traversal_options
        let traversal_option = traversal_options
            .get(self.traversal_option)
            .ok_or_else(|| {
                PluginError::Generic(format!(
                    "Invalid traversal option index: {}",
                    self.traversal_option
                ))
            })?;
        // Step 2: Execute that traversal // update the message trace

        thread.graph_traces.push(GraphTrace {
            message_idx: usr_msg_idx,
            embedding_options: embedding_options.clone(),
            traversal_options: traversal_options.clone(),
            selected_trace_idx: Some(self.traversal_option),
            feedback: false,
        });

        let trav_result = traversal_option.execute(thread, dag, chat_session)?;
        Ok(trav_result)
    }
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
/// If none of the provided traversal paths are determined to satisfy the users intent
/// but there is an embedding provided node that a path *should* exist but does not,
/// this creates a direct path to that node from the current node.
pub struct ConnectToNode {
    /// The index of the embedding provided node
    pub target_node: usize,
}

impl ConnectToNode {
    pub fn execute(
        &self,
        usr_msg_idx: usize,
        current_node: NodeIndex,
        embedding_options: &EmbeddingOptions,
        thread: &mut ThreadStorage,
        dag: &mut Dag<Node, Edge>,
        chat_session: &mut ChatSession,
    ) -> Result<String, PluginError> {
        // Step 1: Get the target node index from embedding options
        let target_node = NodeIndex::new(self.target_node);

        // Step 2: Create edge from current node to target node
        let edge_index = dag
            .add_edge(current_node, target_node, Edge { scent: 1.0 })
            .map_err(|e| {
                PluginError::Generic(format!(
                    "Connect to Node Failed to add edge: {} -- from: {:?} to: {:?}",
                    e, current_node, target_node
                ))
            })?;

        // Step 3: Create a traversal path from current node to target node from this new edge
        let new_path: TraversalOption = TraversalOption {
            edges: vec![edge_index],
        };

        let chosen_path: ChoosePath = ChoosePath {
            traversal_option: 0,
        };

        // Step 4: Execute the new path to target node
        let result = chosen_path.execute(
            usr_msg_idx,
            &vec![new_path],
            embedding_options,
            dag,
            thread,
            chat_session,
        )?;

        Ok(result)
    }
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
/// When the user asks for something vague or uncharted in the graph, a new path and nodes can be created
/// that captures this concept. Upon calling this tool, a number of extra tools
/// will be added to the context, and a tool call create_node will be called by the LLM.
pub struct PlanPath {
    pub description: String,
}

impl PlanPath {
    pub fn execute(
        &self,
        thread_storage: &mut ThreadStorage,
        chat_session: &mut ChatSession,
    ) -> Result<String, PluginError> {
        // Step 1: Call the LLM and force it to use the create_new_path tool
        let ctx_message = if !thread_storage.tools_in_ctx {
            let mut tools = mcp_tools_split()?;
            // recursively set "required" to empty array - removing requirements from all tool schemas
            fn set_empty_required(value: &mut serde_json::Value) {
                if let serde_json::Value::Object(obj) = value {
                    if obj.contains_key("required") {
                        obj.insert("required".to_string(), serde_json::Value::Array(vec![]));
                    }
                    for (_, v) in obj.iter_mut() {
                        set_empty_required(v);
                    }
                } else if let serde_json::Value::Array(arr) = value {
                    for item in arr.iter_mut() {
                        set_empty_required(item);
                    }
                }
            }
            tools.iter_mut().for_each(set_empty_required);
            let full_tools_str =
                serde_json::to_string(&tools).map_err(|e| PluginError::Json(e.to_string()))?;

            thread_storage.tools_in_ctx = true;
            format!(
                "You asked to plan a path.
                Please create simple, efficient, generalizable (where possible) nodes. Consider how it might be used in the future for similar (but slightly different) requests.
                **Rules**
                1. The **root node** defines your central purpose—keep it top of mind.
                2. **Stable arguments only**: don’t hard‑code values that change each request (ie. if you want to fetch messages from slack, don't hardcode the number of messages to summarize
                in the tool call node for that).
                3. **Graph coherency**: new nodes/paths must align strictly with user intent.
                4. If the user asks you to do something, do not stop at creating a simple sub-intention node, keep going until the entire user request is fulfilled.
                5. If the user issues a correction to you (i.e. 'thats wrong', 'no, thats not what I meant', etc.), choose a path that best backtracks (or use jump_to_root) to where you went wrong and then continue from there.
                Here are the tools you can attach to the node if you want to make a tool node: {}",
                full_tools_str
            )
        } else {
            "You asked to plan a path.
                Please create simple, efficient, generalizable (where possible) nodes. Consider how it might be used in the future for similar (but slightly different) requests.
                **Rules**
                1. The **root node** defines your central purpose—keep it top of mind.
                2. **Stable arguments only**: don’t hard‑code values that change each request (ie. if you want to fetch messages from slack, don't hardcode the number of messages to summarize
                in the tool call node for that).
                3. **Graph coherency**: new nodes/paths must align strictly with user intent.
                4. If the user asks you to do something, do not stop at creating a simple sub-intention node, keep going until the entire user request is fulfilled.
                5. If the user issues a correction to you (i.e. 'thats wrong', 'no, thats not what I meant', etc.), choose a path that best backtracks (or use jump_to_root) to where you went wrong and then continue from there.
                The tools have already been added to the context, please reference the first plan_path call.".to_string()
        };
        chat_session.add_tool(&CREATE_NEW_PATH_TOOL.clone().to_string())?;
        chat_session
            .set_tool_choice(Some(&ToolSelection::Forced("create_new_path".to_string())))?;

        Ok(ctx_message)
    }
}

pub fn handle_create_new_path(
    usr_msg_idx: usize,
    thread_storage: &mut ThreadStorage,
    embedding_options: &EmbeddingOptions,
    response: ChatCompletion,
    chat_session: &mut ChatSession,
    dag: &mut Dag<Node, Edge>,
    create_new_path_schema: serde_json::Value,
) -> Result<(Option<String>, String), PluginError> {
    // Implement the logic to fix tool calls here
    let full_response = response
        .choices
        .first()
        .ok_or_else(|| PluginError::ChatCompletion("No response choices".to_string()))?;

    let message = full_response.message.clone();

    // Get tool call ID if tool calls exist and there are any
    let tool_call_id = message
        .tool_calls
        .as_ref()
        .and_then(|tool_calls| tool_calls.first())
        .map(|tool_call| tool_call.id.clone());

    // Step 2: Add the new node to the graph
    let new_path: TraversalOption;
    if let Some(tool_calls) = &message.tool_calls {
        let tool_call = tool_calls.first().unwrap();

        let mut arguments = tool_call.function.arguments.clone();
        if arguments.is_empty() {
            arguments = "{}".to_string();
        }

        let arguments: CreateNewPath = match serde_json::from_str(&arguments) {
            Ok(c) => c,
            Err(_) => serde_json::from_str(&fix_tool_call(
                tool_call,
                chat_session,
                usr_msg_idx as u64 + 1,
                create_new_path_schema.clone(),
            )?)
            .map_err(|e| PluginError::Json(format!("Failed to fix tool call input: {e}")))?,
        };
        new_path = arguments.execute(thread_storage.current_node, dag)?;
    } else {
        return Err(PluginError::ChatCompletion(
            "Failed to call create_new_path".to_string(),
        ));
    }

    let chosen_path: ChoosePath = ChoosePath {
        traversal_option: 0,
    };

    let current_idx = thread_storage.current_node;
    // Step 3: Traverse the graph from current node to new node (target)
    match chosen_path.execute(
        usr_msg_idx,
        &vec![new_path.clone()],
        embedding_options,
        dag,
        thread_storage,
        chat_session,
    ) {
        Ok(result) => Ok((tool_call_id, result)),
        Err(err) => {
            thread_storage.current_node = current_idx;
            new_path.edges.iter().rev().try_for_each(|edge| {
                if let Some((_start, end)) = dag.edge_endpoints(*edge) {
                    let _ = dag.remove_edge(*edge);

                    // delete the new node
                    dag.remove_node(end);
                    let vec_db = connect_db("nb_data")?;
                    vec_db.delete("nodes", &format!("node_idx = {}", end.index()))?;
                }
                Ok(())
            })?;
            let new_msg = format!("Error creating or traversing the path: {}. The nodes in the path have been deleted.
                Please generate a new plan with different node configurations to accomplish the goal.
                Consider different approaches, tools, or more specific descriptions. It could be that you need to remember to ask the user for more information.", err);
            let msg = Message {
                role: "tool".to_string(),
                content: ContentType::Single(MessageContent::Content(new_msg.clone())),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
            };
            chat_session.add_message(&msg)?;
            thread_storage.messages.push(msg);
            let resp = chat_session.send()?;
            let fr = resp
                .choices
                .first()
                .ok_or_else(|| PluginError::ChatCompletion("No response choices".to_string()))?;

            let message = fr.message.clone();
            thread_storage.messages.push(Message {
                role: message.role,
                content: ContentType::Single(MessageContent::Content(
                    message.content.clone().unwrap_or_default(),
                )),
                tool_calls: message.tool_calls.clone(),
                tool_call_id: None,
            });
            handle_create_new_path(
                usr_msg_idx,
                thread_storage,
                embedding_options,
                resp,
                chat_session,
                dag,
                create_new_path_schema,
            )
        }
    }
}

fn fix_tool_call(
    tool_call: &ToolCallUsage,
    chat_session: &mut ChatSession,
    pre_tool_call_idx: u64,
    mut schema: serde_json::Value,
) -> Result<String, PluginError> {
    schema.as_object_mut().unwrap().remove("examples");

    let schema = serde_json::json!({
        "name": "root",
        "strict": true,
        "schema": schema
    })
    .to_string();
    let schema_chat_session = chat_session.fork_at(pre_tool_call_idx)?;
    schema_chat_session.set_response_schema(Some(&schema))?;
    schema_chat_session.remove_all_tools()?;
    schema_chat_session.disable_streaming();
    schema_chat_session.set_tool_choice(None)?;
    schema_chat_session.add_message(&Message {
        role: "assistant".to_string(),
        content: ContentType::Single(MessageContent::Content(format!(
            "Let me try to construct valid JSON to call the function: {} with the arguments: {}",
            tool_call.function.name, tool_call.function.arguments
        ))),
        tool_call_id: None,
        tool_calls: None,
    })?;
    let response = schema_chat_session.send()?;
    let choice = response
        .choices
        .first()
        .ok_or_else(|| PluginError::ChatCompletion("No schema response generated".to_string()))?;

    let fixed_tool_usage = ToolCallUsage {
        function: FunctionUsage {
            name: tool_call.function.name.clone(),
            arguments: choice.message.content.as_ref().unwrap().clone(),
        },
        id: tool_call.id.clone(),
        tool_type: tool_call.tool_type.clone(),
    };

    // Update the tool call in the chat session's messages
    let mut messages = chat_session.messages();
    'm: for message in messages.iter_mut().rev() {
        if let Some(ref mut tool_calls) = &mut message.tool_calls {
            for tool_call in tool_calls.iter_mut() {
                if tool_call.id == fixed_tool_usage.id {
                    *tool_call = fixed_tool_usage.clone();
                    break 'm;
                }
            }
        }
    }

    // Replace the message in the chat session
    chat_session.set_messages(&messages)?;
    Ok(fixed_tool_usage.function.arguments)
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateNewPath {
    /// The ordered sequence of nodes to create. Each node represents a step in the path that will be traversed.
    pub path: Vec<CreateNewNode>,
}

impl CreateNewPath {
    pub fn execute(
        &self,
        mut current_node_idx: NodeIndex,
        dag: &mut Dag<Node, Edge>,
    ) -> Result<TraversalOption, PluginError> {
        let mut edges = Vec::new();
        for node in &self.path {
            let (new_node, edge) = node.execute(current_node_idx, dag)?;
            current_node_idx = new_node;
            edges.push(edge);
        }
        Ok(TraversalOption { edges })
    }
}

/// Creates a new node in the graph.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateNewNode {
    /// A brief description of the node's purpose
    pub description: String,
    /// The specific kind of agent node to create
    pub kind: AgentNode,
    /// The action type that defines how this node interacts with the system
    pub action: ActionType,
}

impl CreateNewNode {
    pub fn execute(
        &self,
        current_node_idx: NodeIndex,
        dag: &mut Dag<Node, Edge>,
    ) -> Result<(NodeIndex, EdgeIndex), PluginError> {
        let mut new_node = Node {
            description: self.description.clone(),
            root: false,
            mutable: true,
            stoppable: true,
            kind: NodeKind::Agent(self.kind.clone()),
            weight: 1.0,
            action: self.action.clone(),
            created_at: None,
            created_via: None,
        };

        if let NodeKind::Agent(AgentNode::ToolCall {
            tool,
            needs_additional_input_generation,
            hardcoded_args,
            ..
        }) = &mut new_node.kind
        {
            let hardcoded_args_json: serde_json::Value = serde_json::from_str(hardcoded_args)
                .map_err(|e| {
                    log(
                        Level::Error,
                        &format!("Invalid JSON in hardcoded_args: {}", hardcoded_args),
                    );
                    PluginError::Json(format!("Invalid hardcoded args JSON: {}", e))
                })?;

            // Function to check if a JSON value contains "<CONC_DYN>"
            fn contains_conc_dyn(value: &serde_json::Value) -> bool {
                match value {
                    serde_json::Value::String(s) if s == "<CONC_DYN>" => true,
                    serde_json::Value::Object(obj) => obj.values().any(contains_conc_dyn),
                    serde_json::Value::Array(arr) => arr.iter().any(contains_conc_dyn),
                    _ => false,
                }
            }

            // Check if there's a mismatch between needs_additional_input_generation and presence of <CONC_DYN>
            let has_dyn_placeholders = contains_conc_dyn(&hardcoded_args_json);

            if !*needs_additional_input_generation && has_dyn_placeholders {
                // If node says it doesn't need dynamic input but has <CONC_DYN> placeholders, raise an error
                *needs_additional_input_generation = true;
            }

            if *needs_additional_input_generation {
                // we need to generate additional input, so we need to fetch the full tools list
                // then find the tool in the full list, then check the hardcoded args and remove them
                // from the json
                let split_tools = mcp_tools_split()?;
                for t in split_tools {
                    let name = t["function"]["name"].as_str().unwrap().to_string();
                    if name == *tool {
                        // Get the parameter schema for this tool
                        let schema = t["function"]["parameters"].clone();

                        // Parse the hardcoded args into a JSON value
                        let mut hardcoded_args_json: serde_json::Value =
                            serde_json::from_str(hardcoded_args).map_err(|e| {
                                log(Level::Error, &format!("Invalid JSON: {}", hardcoded_args));
                                PluginError::Json(format!("Invalid hardcoded args JSON: {}", e))
                            })?;

                        // Create a filtered schema that removes properties that already have values in hardcoded_args
                        let mut filtered_schema = schema.clone();

                        // Function to filter schema properties based on hardcoded args
                        fn filter_schema_recursively(
                            schema: &mut serde_json::Value,
                            hardcoded_args: &serde_json::Value,
                            path: Vec<String>,
                        ) {
                            // Handle objects (most common case)
                            if let (Some(schema_obj), Some(args_obj)) = (
                                schema.get_mut("properties").and_then(|p| p.as_object_mut()),
                                hardcoded_args.as_object(),
                            ) {
                                // Get keys to remove
                                let mut keys_to_remove = Vec::new();

                                // Process each property in the schema
                                for (prop_name, prop_schema) in schema_obj.iter_mut() {
                                    if let Some(arg_value) = args_obj.get(prop_name) {
                                        if arg_value.is_object() && prop_schema.is_object() {
                                            // Recursive case: both are objects, continue filtering
                                            let mut new_path = path.clone();
                                            new_path.push(prop_name.clone());
                                            filter_schema_recursively(
                                                prop_schema,
                                                arg_value,
                                                new_path,
                                            );
                                        } else if arg_value.is_string()
                                            && arg_value.as_str() == Some("<CONC_DYN>")
                                        {
                                            // Keep this property as it needs dynamic input
                                            continue;
                                        } else {
                                            // Non-dynamic value provided in hardcoded args, mark for removal
                                            keys_to_remove.push(prop_name.clone());
                                        }
                                    }
                                }

                                // Remove properties that have hardcoded values
                                for key in &keys_to_remove {
                                    schema_obj.remove(key);
                                }
                                for key in keys_to_remove {
                                    // Also remove from required array if it exists
                                    // Look in the parent schema for the required array
                                    if let Some(required) = schema.get_mut("required") {
                                        if let Some(required_arr) = required.as_array_mut() {
                                            required_arr.retain(|v| {
                                                if let Some(s) = v.as_str() {
                                                    s != key
                                                } else {
                                                    true
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                        }

                        // Start filtering from the root with an empty path
                        filter_schema_recursively(
                            &mut filtered_schema,
                            &hardcoded_args_json,
                            Vec::new(),
                        );

                        // Remove <CONC_DYN> values from hardcoded_args_json
                        fn remove_conc_dyn_recursively(json: &mut serde_json::Value) {
                            if let serde_json::Value::Object(obj) = json {
                                // Collect keys to remove
                                let keys_to_remove: Vec<String> = obj
                                    .iter()
                                    .filter_map(|(k, v)| {
                                        if let serde_json::Value::String(s) = v {
                                            if s == "<CONC_DYN>" {
                                                return Some(k.clone());
                                            }
                                        }
                                        None
                                    })
                                    .collect();

                                // Remove the collected keys
                                for key in keys_to_remove {
                                    obj.remove(&key);
                                }

                                // Recursively process remaining objects
                                for (_, v) in obj.iter_mut() {
                                    remove_conc_dyn_recursively(v);
                                }
                            } else if let serde_json::Value::Array(arr) = json {
                                for item in arr.iter_mut() {
                                    remove_conc_dyn_recursively(item);
                                }
                            }
                        }

                        // Remove <CONC_DYN> values from hardcoded_args_json
                        remove_conc_dyn_recursively(&mut hardcoded_args_json);

                        // log(
                        //     Level::Info,
                        //     &format!(
                        //         "Filtered schema: {}\nHardcoded args: {}\n",
                        //         serde_json::to_string_pretty(&filtered_schema).unwrap(),
                        //         serde_json::to_string_pretty(&hardcoded_args_json).unwrap()
                        //     ),
                        // );

                        new_node.kind = NodeKind::Agent(AgentNode::ToolCall {
                            tool: tool.clone(),
                            needs_additional_input_generation: *needs_additional_input_generation,
                            hardcoded_args: hardcoded_args_json.to_string(),
                            filtered_schema,
                        });

                        break;
                    }
                }
            }
        }
        let node_index = dag.add_node(new_node.clone());
        embed_node(&new_node, "nb_data", node_index)?;
        // create edge from current node to new node
        let edge_index = dag
            .add_edge(current_node_idx, node_index, Edge { scent: 1.0 })
            .map_err(|e| {
                PluginError::Generic(format!(
                    "Create New Node Failed to add edge: {} -- from: {:?} to: {:?}",
                    e, current_node_idx, node_index
                ))
            })?;
        Ok((node_index, edge_index))
    }
}

pub fn embedding_options(
    embedding_client: &Client,
    model_name: &str,
    db_path: &str,
    input: &str,
) -> Result<Vec<NodeIndex>, PluginError> {
    let vec_db = connect_db(db_path)?;
    let search_config = SimilaritySearchConfig {
        limit: Some(5),
        threshold: Some(0.7),
        fields_returned: vec!["node_idx".to_string()],
        where_clause: None,
        include_embeddings: Some(false),
    };

    let mut search_results = vec_db
        .similarity_search(&search_config, embedding_client, model_name, "nodes", input)?
        .pop()
        .ok_or_else(|| PluginError::Unexpected("No search results returned".to_string()))?;

    // log(
    //     Level::Info,
    //     &format!("Search results: {:#?}", search_results),
    // );

    if search_results.columns.is_empty() {
        return Ok(vec![]);
    }

    let indices = search_results.columns.swap_remove(0);

    // log(Level::Info, &format!("Indices results: {:#?}", indices));

    indices
        .iter()
        .map(|idx| Ok(NodeIndex::new(serde_json::from_str(idx)?)))
        .collect::<Result<Vec<NodeIndex>, serde_json::Error>>()
        .map_err(|e| PluginError::Json(format!("Unable to create node index: {}", e)))
}

pub fn path_traversal_options(
    thread: &ThreadStorage,
    options: &EmbeddingOptions,
    graph: &Dag<Node, Edge>,
) -> Result<TraversalOptions, PluginError> {
    use daggy::petgraph::algo::{has_path_connecting, simple_paths::all_simple_paths};
    let mut path_options = Vec::new();

    // given some potential goal nodes, check if we have a path to it
    // if we do, get 3 potential paths from start node to goal node and
    // make a TraversalOption out of each
    for potential_goal in options {
        // log(
        //     Level::Info,
        //     &format!(
        //         "Checking path to goal node: (Current:{:?}) (Destination:{:?}) {:#?}",
        //         thread.current_node,
        //         potential_goal,
        //         Dot::new(graph.graph())
        //     ),
        // );
        let has_path =
            has_path_connecting(graph.graph(), thread.current_node, *potential_goal, None);
        if has_path {
            // log(Level::Info, "Found path");
            // get 3 potential paths from start node to goal node
            // WARNING: A single path can be found in O(Vertex + Edge)
            // time but the number of simple paths in a graph can be very large, e.g.
            // in the complete graph of order O(n!)
            let paths_as_nodes = all_simple_paths::<Vec<_>, _, RandomState>(
                &graph.graph(),
                thread.current_node,
                *potential_goal,
                0,
                None,
            )
            .take(3)
            .collect::<Vec<_>>();

            let traversal_options_to_goal = paths_as_nodes
                .iter()
                .map(|path| TraversalOption {
                    edges: path
                        .windows(2)
                        .map(|pair| graph.find_edge(pair[0], pair[1]).unwrap())
                        .collect(),
                })
                .collect::<Vec<_>>();

            path_options.extend(traversal_options_to_goal);
        } else {
            // log(Level::Info, "NO PATH");
        }
    }

    // add all children
    let child_paths = graph
        .children(thread.current_node)
        .iter(graph)
        .map(|child| {
            let edge = graph.find_edge(thread.current_node, child.1).unwrap();
            TraversalOption { edges: vec![edge] }
        })
        .collect::<Vec<_>>();

    path_options.extend(child_paths);
    path_options.extend(get_parent_paths(thread, graph, 3));

    Ok(path_options)
}

pub fn get_parent_paths(
    thread: &ThreadStorage,
    graph: &Dag<Node, Edge>,
    max_depth: usize,
) -> Vec<TraversalOption> {
    let mut result = Vec::new();
    let current_node = thread.current_node;

    // Helper function to recursively build paths
    fn build_parent_paths(
        graph: &Dag<Node, Edge>,
        node: NodeIndex,
        current_path: Vec<EdgeIndex>,
        result: &mut Vec<TraversalOption>,
        depth: usize,
        max_depth: usize,
    ) {
        // Add the current path as a TraversalOption if it's not empty
        if !current_path.is_empty() {
            // Create a new TraversalOption with the edges reversed (to follow from parent to child)
            let mut path_edges = current_path.clone();
            path_edges.reverse();
            result.push(TraversalOption { edges: path_edges });
        }

        // Stop recursion if we've reached the maximum depth
        if depth >= max_depth {
            return;
        }

        // Get all parents of the current node
        let parents = graph.parents(node);

        // Iterate through each parent and build paths
        for (edge, parent) in parents.iter(graph) {
            // Check if the parent is a root node - if so, skip it
            if let Some(parent_node) = graph.node_weight(parent) {
                if parent_node.root {
                    continue; // Don't include root nodes in the path
                }
            }

            let mut new_path = current_path.clone();
            new_path.push(edge);

            // Recursively build paths from this parent
            build_parent_paths(graph, parent, new_path, result, depth + 1, max_depth);
        }
    }

    // Start building paths from the current node
    build_parent_paths(graph, current_node, Vec::new(), &mut result, 0, max_depth);

    result
}

impl Node {
    // Should be building a context string along the execution path and returning
    pub fn execute(
        &self,
        context: &mut String,
        chat_session: &mut ChatSession,
    ) -> Result<String, PluginError> {
        // Execute the node's action or add the nodes goal to the context
        // context.push_str(&format!("Node Action Taken: {:?}", self.action));
        match &self.kind {
            NodeKind::System(SystemNode::RootIntention { text }) => {
                context.push('\n');
                context.push_str(&format!("Traversed node: {} | ", self.description));
                context.push_str(&format!("Node text: {} | ", text));
            }
            NodeKind::System(SystemNode::InitDirective { text }) => {
                context.push('\n');
                context.push_str(&format!("Traversed node: {} | ", self.description));
                context.push_str(&format!("Node text: {} | ", text));
            }
            NodeKind::Agent(AgentNode::SubIntention {
                text,
                utility: _,
                status: _,
            }) => {
                // TODO: figure out what status should do?
                context.push('\n');
                context.push_str(text);
            }
            NodeKind::Agent(AgentNode::ToolCall {
                tool,
                needs_additional_input_generation,
                hardcoded_args,
                filtered_schema,
            }) => {
                let real_func_name = tool
                    .split("_sub_tool_")
                    .next()
                    .map(|s| s.to_string())
                    .unwrap_or(tool.clone());
                // TODO: once we decide on how to handle dyn args,
                // will need to first call the LLM to generate the inputs
                // then fill the inputs in, then call the plugin
                if !needs_additional_input_generation {
                    match call_plugin(&real_func_name, hardcoded_args) {
                        Ok(res) => {
                            context.push('\n');
                            context.push_str(&res);
                        }
                        Err(e) => {
                            context.push_str("\nError:");
                            context.push_str(&e.to_string());
                        }
                    }
                } else {
                    let args_as_json: serde_json::Value = serde_json::from_str(hardcoded_args)
                        .map_err(|e| PluginError::Json(e.to_string()))?;
                    let full_args = generate_inputs(
                        chat_session,
                        filtered_schema.clone(),
                        context,
                        &args_as_json,
                    )?;
                    match call_plugin(&real_func_name, &full_args.to_string()) {
                        Ok(res) => {
                            context.push('\n');
                            context.push_str(&res);
                        }
                        Err(e) => {
                            context.push_str("\nError:");
                            context.push_str(&e.to_string());
                        }
                    }
                }
            }
            NodeKind::Agent(AgentNode::Context {
                source,
                summary,
                vector_id: _,
            }) => {
                context.push('\n');
                context.push_str(&format!(
                    "\n<context>\n<source>{}</source>\n<summary>{}</summary>\n</context>",
                    source, summary
                ));
            }
        }
        context.push('\n');
        context.push_str(&format!("Node Action Taken: {:?} ", self.action));
        Ok(context.clone())
    }
}

/// Author of a node addition (agent vs. human)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Author {
    Agent,
    Human,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    /// Traverses child nodes in the graph; default action for intent nodes
    /// that instructs the system to follow connections to child nodes
    TraverseChildren,

    /// Creates a plan for achieving a goal; default action for new subgoal nodes
    /// that need further planning or breakdown
    Plan,

    /// Performs evaluation and pruning of the graph; reflection step that passes
    /// in the whole DAG and prunes low weighted edges/nodes to optimize the graph
    EvalPrune,

    /// Writes to the internal system state; default action for ToolCall nodes
    /// that modify internal data structures or state
    IntWrite,

    /// Reads from internal system state; used to query the internal world
    /// to retrieve stored data or context
    IntRead,

    /// Executes an external write action; used for operations that change
    /// the state of external systems or resources
    ExtWrite,

    /// Queries the external world; used for operations that gather data
    /// from external systems without modifying them
    ExtRead,

    /// Adds persistent memory to the graph and context; used to store
    /// important information for future reference
    AddMemory,

    /// Explicitly does nothing; used when a node should exist in the graph
    /// but not perform any action when traversed
    Noop,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Active,
    Blocked,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EdgeData {
    weight: f32,
}

pub fn init_graph() -> Result<Dag<Node, Edge>, PluginError> {
    let graph_path = "nb/dags/intent_dag.json";

    let graph_path = Path::new(&graph_path);
    if let Some(parent) = Path::new(&graph_path).parent() {
        fs::create_dir_all(parent).map_err(|e| {
            PluginError::Fs(FsError::Other(format!(
                "Failed to create directory for thread file: {}",
                e
            )))
        })?;
    }

    if graph_path.exists() {
        let file = std::fs::File::open(graph_path)
            .map_err(|e| PluginError::Unexpected(format!("Failed to open graph file: {}", e)))?;
        let dag: Dag<Node, Edge> = serde_json::from_reader(file)
            .map_err(|e| PluginError::Unexpected(format!("Failed to deserialize graph: {}", e)))?;
        return Ok(dag);
    } else {
        // Create the graph file
        let _ = std::fs::File::create(graph_path)
            .map_err(|e| PluginError::Unexpected(format!("Failed to create graph file: {}", e)))?;
    }

    let root = Node::new(
        "Root Intention".to_string(),
        true,
        NodeKind::System(SystemNode::RootIntention {
            text: "Be a helpful employee to Nullborn Industries".to_string(),
        }),
        100.0,
        false,
        ActionType::TraverseChildren,
    );
    let child1 = Node::new(
        "Build Graph Directive".to_string(),
        true,
        NodeKind::System(SystemNode::InitDirective {
            text: "Build a graph of sub-intentions to support the root intention".to_string(),
        }),
        100.0,
        false,
        ActionType::TraverseChildren,
    );
    let child2 = Node::new(
        "Build Memory Directive".to_string(),
        true,
        NodeKind::System(SystemNode::InitDirective {
            text: "Build internal memory to support the root intention".to_string(),
        }),
        100.0,
        false,
        ActionType::TraverseChildren,
    );

    let mut dag = Dag::new();

    let root_idx = dag.add_node(root.clone());
    let child1_idx = dag.add_node(child1.clone());
    let child2_idx = dag.add_node(child2.clone());

    // delete nb_data if it exists
    if Path::new("nb_data").exists() {
        std::fs::remove_dir_all("nb_data").map_err(|e| {
            PluginError::Unexpected(format!("Failed to delete nb_data directory: {}", e))
        })?;
    }
    // embed nodes to vector db
    embed_node(&child1, "nb_data", child1_idx)?;
    embed_node(&child2, "nb_data", child2_idx)?;
    embed_node(&root, "nb_data", root_idx)?;

    dag.add_edge(root_idx, child1_idx, Edge { scent: 1.0 })
        .map_err(|e| {
            PluginError::Unexpected(format!(
                "Failed to add edge: {} -- from: {:?} to: {:?}",
                e, root_idx, child1_idx
            ))
        })?;
    dag.add_edge(child1_idx, child2_idx, Edge { scent: 1.0 })
        .map_err(|e| {
            PluginError::Unexpected(format!(
                "Failed to add edge: {} -- from: {:?} to: {:?}",
                e, child1_idx, child2_idx
            ))
        })?;

    // save the graph to a file
    let file = std::fs::File::create(graph_path)
        .map_err(|e| PluginError::Unexpected(format!("Failed to create graph file: {}", e)))?;
    serde_json::to_writer_pretty(file, &dag)
        .map_err(|e| PluginError::Unexpected(format!("Failed to serialize graph: {}", e)))?;
    Ok(dag)
}

fn embed_node(node: &Node, db_path: &str, node_idx: NodeIndex) -> Result<bool, PluginError> {
    let embedding_config = EmbeddingConfig::try_from_env_var("EMBEDDING_CONFIG")
        .map_err(|e| PluginError::EnvVar(format!("Failed to load embedding config: {}", e)))?;

    let embedding_client = new_client(&embedding_config.base_url, &embedding_config.api_key)?;

    let vec_db = connect_db(db_path)?;

    let schema_json_str = serde_json::to_string(&arrow_schema::Schema::new(vec![
        arrow_schema::Field::new("description", arrow_schema::DataType::Utf8, false),
        arrow_schema::Field::new("node_idx", arrow_schema::DataType::UInt64, false),
        arrow_schema::Field::new(
            "embeddings",
            arrow_schema::DataType::FixedSizeList(
                std::sync::Arc::new(arrow_schema::Field::new(
                    "item",
                    arrow_schema::DataType::Float32,
                    true,
                )),
                768,
            ),
            false,
        ),
    ]))
    .unwrap();

    // Create nodes table if it doesn't exist
    let table_names = vec_db.get_table_names()?;
    if !table_names.contains(&"nodes".to_string()) {
        vec_db.create_table("nodes", &schema_json_str)?;
    }
    let descr_string: String = match &node.kind {
        NodeKind::System(SystemNode::RootIntention { text })
        | NodeKind::System(SystemNode::InitDirective { text })
        | NodeKind::Agent(AgentNode::SubIntention { text, .. }) => text.clone(), // clone the existing String

        NodeKind::Agent(AgentNode::ToolCall {
            tool,
            hardcoded_args,
            ..
        }) => {
            format!("Tool: {}, args: {}", tool, hardcoded_args)
        }

        NodeKind::Agent(AgentNode::Context { summary, .. }) => summary.clone(),
    };

    let text_column =
        serde_json::Value::Array(vec![
            format!("{}\n{}", node.description, descr_string).into()
        ])
        .to_string();
    let index_column = serde_json::Value::Array(vec![node_idx.index().into()]).to_string();

    // Embed the node
    let success = vec_db.add(
        "nodes",
        embedding_client,
        &embedding_config.model_name,
        &[text_column, index_column],
        0, // Embed the 0th column
        Some("node_idx"),
    )?;

    Ok(success)
}
