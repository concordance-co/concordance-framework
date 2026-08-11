use daggy::{Dag, NodeIndex, Walker};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::inlined_schema_for;
use shared::types::{ThreadInfo, ThreadSummary};

use crate::exports::plugin::injector::guest::PluginError;
use crate::graph_utils::{
    get_parent_paths, handle_create_new_path, path_traversal_options, AgentNode, ChoosePath,
    ConnectToNode, CreateNewPath, Edge, EmbeddingOptions, GraphTrace, Node, NodeKind, PlanPath,
    TraversalOption, TraversalOptions, CONNECT_TO_NODE_TOOL, JUMP_TO_ROOT_TOOL,
    PATH_SELECTION_TOOL, PLAN_PATH_TOOL,
};
use crate::llm_utils::ThreadStorage;
use crate::plugin::injector::error::FsError;
use crate::plugin::injector::host::{log, Level};
// use crate::plugin::injector::host::{log, Level};
use crate::plugin::injector::open_a_i_like::{
    ChatCompletion, ChatSession, ContentType, Message, MessageContent,
};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, path::Path};

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct NBChatRequest {
    pub message: String,
    pub graph_id: Option<String>,
    pub system_context: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct NBChatResponse {
    pub response: String,
    pub thread_id: String,
    // pub goals: String,
}

pub fn process_message(
    chat_request: &NBChatRequest,
    chat_session: &mut ChatSession,
    mut embedding_options: EmbeddingOptions,
    thread_id: &str,
    dag: &mut Dag<Node, Edge>,
) -> Result<NBChatResponse, PluginError> {
    // Load thread storage
    let thread_file_path = format!("nb/threads/thread-{}.json", thread_id);

    let mut thread_storage: ThreadStorage = if Path::new(&thread_file_path).exists() {
        let thread_data = fs::read_to_string(&thread_file_path).map_err(|e| {
            PluginError::Fs(FsError::Other(format!("Failed to read thread file: {}", e)))
        })?;
        serde_json::from_str(&thread_data)
            .map_err(|e| PluginError::Json(format!("Invalid thread data: {}", e)))?
    } else {
        return Err(PluginError::Fs(FsError::Other(
            "Thread file not found".to_string(),
        )));
    };
    log(
        Level::Info,
        &format!(
            "Current node (message_utils): {:?}",
            thread_storage.current_node
        ),
    );

    // Remove the current node from embedding options
    embedding_options.retain(|option| *option != thread_storage.current_node);

    if embedding_options.is_empty() {
        let _ = chat_session.remove_tool(&CONNECT_TO_NODE_TOOL.clone().to_string())?;
    }

    let init_message = if thread_storage.current_node == NodeIndex::new(0) {
        let message = process_init_message(
            &mut thread_storage,
            &embedding_options,
            1,
            dag,
            chat_session,
        )?;
        Some(message)
    } else {
        None
    };

    if thread_storage.current_node == NodeIndex::new(2) {
        let _ = chat_session.remove_tool(&JUMP_TO_ROOT_TOOL.clone().to_string())?;
    }

    let traversal_options = path_traversal_options(&thread_storage, &embedding_options, dag)?;
    if traversal_options.is_empty() {
        let _ = chat_session.remove_tool(&PATH_SELECTION_TOOL.clone().to_string())?;
    } else {
        let _ = chat_session.add_tool(&PATH_SELECTION_TOOL.clone().to_string())?;
    }

    // Build message to send to LLM based on traversal options
    let message_context =
        build_path_selection_context(&embedding_options, &traversal_options, dag, init_message);

    let full_message = Message {
        role: "user".to_string(),
        content: ContentType::Single(MessageContent::Content(format!(
            "<user_input>{}</user_input>\n\n<context_from_intent_graph>\n{}\n</context_from_intent_graph>",
            chat_request.message, message_context
        ))),
        tool_calls: None,
        tool_call_id: None,
    };

    log(
        Level::Info,
        &format!("Sending message to LLM (message_utils): {:?}", full_message),
    );

    chat_session.add_message(&full_message)?;
    thread_storage.messages.push(full_message);
    thread_storage.graph_traces.push(GraphTrace {
        message_idx: thread_storage.messages.len() - 1,
        embedding_options: embedding_options.to_owned(),
        traversal_options: traversal_options.clone(),
        selected_trace_idx: None, // updated in choose_path
        feedback: false,
    });

    let response = chat_session.send()?;

    // Process the LLM's response
    let response_text = process_tool_selection(
        &response,
        chat_session,
        &mut thread_storage,
        &embedding_options,
        dag,
        &traversal_options,
    )?;

    save_thread_storage(&thread_storage, thread_id)?;
    // Save the updated intent graph to disk
    let intent_graph_file_path = "nb/dags/intent_dag.json";

    let serialized_dag = serde_json::to_string_pretty(dag)
        .map_err(|e| PluginError::Json(format!("Failed to serialize intent graph: {}", e)))?;

    fs::write(intent_graph_file_path, serialized_dag).map_err(|e| {
        PluginError::Fs(FsError::Other(format!(
            "Failed to write intent graph file: {}",
            e
        )))
    })?;

    Ok(NBChatResponse {
        response: response_text,
        thread_id: thread_id.to_string(),
    })
}

fn process_init_message(
    thread: &mut ThreadStorage,
    embedding_options: &EmbeddingOptions,
    user_msg_idx: usize,
    dag: &Dag<Node, Edge>,
    chat_session: &mut ChatSession,
) -> Result<String, PluginError> {
    // Create new traversal option
    let trav_options = path_traversal_options(thread, &vec![NodeIndex::new(2)], dag)?;
    let args = ChoosePath {
        traversal_option: 0,
    };
    let res = args.execute(
        user_msg_idx,
        &trav_options,
        embedding_options,
        dag,
        thread,
        chat_session,
    )?;

    let _ = chat_session.add_tool(&PLAN_PATH_TOOL.clone().to_string())?;
    let _ = chat_session.add_tool(&PATH_SELECTION_TOOL.clone().to_string())?;

    Ok(res)
}

fn build_path_selection_context(
    node_options: &[NodeIndex],
    traversal_options: &TraversalOptions,
    dag: &Dag<Node, Edge>,
    init_message: Option<String>,
) -> String {
    // Add embedding options
    let mut context = "<embedding_matches>\n".to_string();
    for node_idx in node_options.iter() {
        if let Some(node) = dag.node_weight(*node_idx) {
            context.push_str(&format!(
                "Node Index: {} | Node Description: {}\n",
                node_idx.index(),
                node.description
            ));
        }
    }
    context.push_str("</embedding_matches>");

    // Add traversal options
    context.push_str("\n<possible_paths>\n");
    for (i, option) in traversal_options.iter().enumerate() {
        context.push_str(&format!(
            "Index {}: Path (scent: {:.2}): {}\n",
            i,
            option.scent(dag),
            option.context(dag)
        ));
    }
    context.push_str("\n</possible_paths>\n");

    if let Some(message) = init_message {
        context.push_str("<previous_traversal>\n");
        context.push_str(&message);
        context.push_str("\n</previous_traversal>\n");
    }
    // Add tool instructions
    context.push_str(
        "\nPlease select an action given the tools you have and the information provided.",
    );
    context
}

fn process_tool_selection(
    tool_response: &ChatCompletion,
    chat_session: &mut ChatSession,
    thread_storage: &mut ThreadStorage,
    embedding_options: &EmbeddingOptions,
    dag: &mut Dag<Node, Edge>,
    traversal_options: &TraversalOptions,
) -> Result<String, PluginError> {
    // last message pushed was from user --> need to have to update graph trace with llm selection
    let user_msg_idx = thread_storage.messages.len() - 1;

    if thread_storage.current_node == NodeIndex::new(2) {
        let _ = chat_session.remove_tool(&JUMP_TO_ROOT_TOOL.to_string());
    } else {
        let _ = chat_session.add_tool(&JUMP_TO_ROOT_TOOL.to_string());
    }
    // Extract the tool call from the response
    let full_response = tool_response
        .choices
        .first()
        .ok_or_else(|| PluginError::ChatCompletion("No response choices".to_string()))?;

    let message = full_response.message.clone();
    log(
        Level::Info,
        &format!("Agent response (message_utils): {:?}", full_response),
    );

    // Get tool call ID if tool calls exist and there are any
    let mut tool_call_id = message
        .tool_calls
        .as_ref()
        .and_then(|tool_calls| tool_calls.first())
        .map(|tool_call| tool_call.id.clone());

    thread_storage.messages.push(Message {
        role: message.role,
        content: ContentType::Single(MessageContent::Content(
            message.content.clone().unwrap_or_default(),
        )),
        tool_calls: message.tool_calls.clone(),
        tool_call_id: None,
    });

    let mut tool_response_str: String;
    if let Some(tool_calls) = &message.tool_calls {
        let tool_call = tool_calls.first().unwrap();
        match tool_call.function.name.as_str() {
            "choose_path" => {
                log(
                    Level::Info,
                    &format!("Executing choose_path with tool call ID: {}", tool_call.id),
                );
                let arguments: ChoosePath =
                    serde_json::from_str(&tool_call.function.arguments.clone()).unwrap();

                if arguments.traversal_option > traversal_options.len() {
                    tool_response_str = format!(
                        "Traversal option {} is out of range",
                        arguments.traversal_option
                    );
                } else {
                    tool_response_str = arguments.execute(
                        user_msg_idx,
                        traversal_options,
                        embedding_options,
                        dag,
                        thread_storage,
                        chat_session,
                    )?;
                }

                if matches!(
                    dag.node_weight(thread_storage.current_node).unwrap().kind,
                    NodeKind::Agent(AgentNode::SubIntention { .. })
                ) {
                    // if we ended on a sub-intention, get child traversal options
                    let path_options = dag
                        .children(thread_storage.current_node)
                        .iter(dag)
                        .map(|child| {
                            let edge = dag.find_edge(thread_storage.current_node, child.1).unwrap();
                            TraversalOption { edges: vec![edge] }
                        })
                        .collect::<Vec<_>>();
                    let _ = chat_session.add_tool(&PATH_SELECTION_TOOL.clone().to_string())?;
                    let _ = chat_session.add_tool(&PLAN_PATH_TOOL.clone().to_string())?;
                    tool_response_str
                        .push_str("### Possible Paths (remember to prioritize longer paths):\n");
                    for (i, option) in path_options.iter().enumerate() {
                        tool_response_str.push_str(&format!(
                            "Index {}: Path (scent: {:.2}): {}\n",
                            i,
                            option.scent(dag),
                            option.context(dag)
                        ));
                    }
                }
            }
            "connect_to_node" => {
                log(
                    Level::Info,
                    &format!(
                        "Executing connect_to_node with tool call ID: {}",
                        tool_call.id
                    ),
                );
                let arguments: ConnectToNode =
                    serde_json::from_str(&tool_call.function.arguments.clone()).unwrap();
                tool_response_str = arguments.execute(
                    user_msg_idx,
                    thread_storage.current_node,
                    embedding_options,
                    thread_storage,
                    dag,
                    chat_session,
                )?;
            }
            "plan_path" => {
                log(
                    Level::Info,
                    &format!("Executing plan_path with tool call ID: {}", tool_call.id),
                );
                let arguments: PlanPath =
                    serde_json::from_str(&tool_call.function.arguments.clone()).unwrap();
                tool_response_str = arguments.execute(thread_storage, chat_session)?;
            }
            "create_new_path" => {
                log(
                    Level::Info,
                    &format!(
                        "Executing create_new_path with tool call ID: {}",
                        tool_call.id
                    ),
                );
                (tool_call_id, tool_response_str) = handle_create_new_path(
                    user_msg_idx,
                    thread_storage,
                    embedding_options,
                    tool_response.clone(),
                    chat_session,
                    dag,
                    inlined_schema_for!(CreateNewPath).to_value(),
                )?;

                chat_session.set_tool_choice(None)?;
            }
            "jump_to_root" => {
                log(
                    Level::Info,
                    &format!("Executing jump_to_root with tool call ID: {}", tool_call.id),
                );
                thread_storage.current_node = NodeIndex::new(2);
                tool_response_str = "Jumped to root\n".to_string();
                let mut path_options = dag
                    .children(thread_storage.current_node)
                    .iter(dag)
                    .map(|child| {
                        let edge = dag.find_edge(thread_storage.current_node, child.1).unwrap();
                        TraversalOption { edges: vec![edge] }
                    })
                    .collect::<Vec<_>>();
                path_options.extend(get_parent_paths(thread_storage, dag, 3));
                let _ = chat_session.add_tool(&PATH_SELECTION_TOOL.clone().to_string())?;
                let _ = chat_session.add_tool(&PLAN_PATH_TOOL.clone().to_string())?;
                tool_response_str
                    .push_str("### Possible Paths (remember to prioritize longer paths):\n");
                for (i, option) in path_options.iter().enumerate() {
                    tool_response_str.push_str(&format!(
                        "Index {}: Path (scent: {:.2}): {}\n",
                        i,
                        option.scent(dag),
                        option.context(dag)
                    ));
                }
            }
            _ => {
                log(
                    Level::Warn,
                    &format!(
                        "Unknown tool call: {:?} with ID: {}",
                        tool_call, tool_call.id
                    ),
                );
                // try again ?
                tool_response_str = "error".to_string();
            }
        }
    } else {
        // log(Level::Warn, "No tool calls found in message");
        return Ok(message.content.unwrap_or_else(|| "".to_string()));
    }

    tool_response_str.push_str(&format!(
        "\n Current node index: {} -- root index: 2",
        thread_storage.current_node.index()
    ));
    // send the response
    // log(
    //     Level::Info,
    //     &format!("Sending response with tool call ID: {:?}", tool_call_id),
    // );
    let msg = Message {
        role: if tool_call_id.is_some() {
            "tool".to_string()
        } else {
            "developer".to_string()
        },
        content: ContentType::Single(MessageContent::Content(tool_response_str.clone())),
        tool_calls: None,
        tool_call_id,
    };
    chat_session.add_message(&msg)?;
    thread_storage.messages.push(msg);
    let tool_processed_response = chat_session.send()?;
    // Check if there are any tools in the processed response
    if let Some(tool_calls) = &tool_processed_response
        .choices
        .first()
        .unwrap()
        .message
        .tool_calls
    {
        if !tool_calls.is_empty() {
            // Recursively process any further tool calls
            // log(
            //     Level::Info,
            //     &format!(
            //         "Detected nested tool calls, processing recursively. First tool call ID: {}",
            //         tool_calls.first().unwrap().id
            //     ),
            // );
            return process_tool_selection(
                &tool_processed_response,
                chat_session,
                thread_storage,
                embedding_options,
                dag,
                traversal_options,
            );
        }
    }

    // No tools in response, extract the final response text
    let final_response = tool_processed_response
        .choices
        .first()
        .map(|choice| choice.message.content.clone().unwrap_or_default())
        .unwrap_or_default();

    // log(
    //     Level::Info,
    //     "Adding final assistant message to thread storage (no tool calls)",
    // );
    thread_storage.messages.push(Message {
        role: "assistant".to_string(),
        content: ContentType::Single(MessageContent::Content(final_response.clone())),
        tool_calls: None,
        tool_call_id: None,
    });

    Ok(tool_response_str)
}

pub fn save_thread_storage(
    thread_storage: &ThreadStorage,
    thread_id: &str,
) -> Result<(), PluginError> {
    let thread_file_path = format!("nb/threads/thread-{}.json", thread_id);

    // Ensure directory exists
    if let Some(parent) = Path::new(&thread_file_path).parent() {
        fs::create_dir_all(parent).map_err(|e| {
            PluginError::Fs(FsError::Other(format!("Failed to create directory: {}", e)))
        })?;
    }

    // Serialize and save the thread storage
    let thread_data = serde_json::to_string_pretty(thread_storage)
        .map_err(|e| PluginError::Json(format!("Failed to serialize thread data: {}", e)))?;

    fs::write(&thread_file_path, thread_data).map_err(|e| {
        PluginError::Fs(FsError::Other(format!(
            "Failed to write thread file from message_utils: {}",
            e
        )))
    })?;
    update_thread_summary(thread_id, None)?;
    Ok(())
}

fn update_thread_summary(thread_id: &str, title: Option<String>) -> Result<(), PluginError> {
    let threads_dir = "nb/threads";
    let summary_path = format!("{}/threads_summary.json", threads_dir);
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Create or update the thread summary
    let mut thread_summary = if Path::new(&summary_path).exists() {
        let summary_data = fs::read_to_string(&summary_path).map_err(|e| {
            PluginError::Fs(FsError::Other(format!(
                "Failed to read summary file: {}",
                e
            )))
        })?;

        serde_json::from_str::<ThreadSummary>(&summary_data).unwrap_or_default()
    } else {
        ThreadSummary::default()
    };
    // Update or add this thread's information
    if let Some(existing_thread) = thread_summary
        .threads
        .iter_mut()
        .find(|t| t.id == thread_id)
    {
        existing_thread.title = title;
        existing_thread.last_message_timestamp = current_time;
    } else {
        thread_summary.threads.push(ThreadInfo {
            id: thread_id.to_string(),
            title,
            last_message_timestamp: current_time,
        });
    }

    // Write the updated summary back to file
    let summary_json = serde_json::to_string(&thread_summary)
        .map_err(|e| PluginError::Json(format!("Failed to serialize thread summary: {}", e)))?;

    fs::write(&summary_path, summary_json).map_err(|e| {
        PluginError::Fs(FsError::Other(format!(
            "Failed to write thread summary file: {}",
            e
        )))
    })?;

    // log(
    //     Level::Info,
    //     &format!("Updated threads summary at {}", summary_path),
    // );
    Ok(())
}
