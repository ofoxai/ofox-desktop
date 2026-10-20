use crate::services::catalog_lifecycle::operation::{self, Action};
use serde_json::{json, Value};
#[tauri::command]
pub async fn get_catalog_copy_state(tool_id: String) -> Result<Value, String> {
    operation::inspect(&tool_id).await
}
#[tauri::command]
pub async fn prepare_catalog_operation(tool_id: String, action: Action) -> Result<Value, String> {
    operation::prepare(&tool_id, action).await
}
#[tauri::command]
pub async fn run_catalog_operation(
    app: tauri::AppHandle,
    operation_id: String,
) -> Result<Value, String> {
    use tauri::Emitter;
    let id = operation_id.clone();
    operation::run(&operation_id, move |stage| {
        let _ = app.emit(
            "catalog-operation-progress",
            json!({"operationId":id,"stage":stage}),
        );
    })
    .await
}
#[tauri::command]
pub fn cancel_catalog_operation(operation_id: String) -> Result<bool, String> {
    operation::cancel(&operation_id)
}
