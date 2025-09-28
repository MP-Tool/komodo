use komodo_client::entities::stats::SystemProcess;
use resolver_api::Resolve;
use serde::{Deserialize, Serialize};

//

#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[response(Vec<SystemProcess>)]
#[error(serror::Error)]
pub struct GetSystemProcesses {}

//
