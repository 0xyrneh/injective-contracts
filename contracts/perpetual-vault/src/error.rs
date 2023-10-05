use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Custom Error val: {val:?}")]
    CustomError { val: String },

    #[error("Failure response from submsg: {0}")]
    SubMsgFailure(String),

    #[error("Unrecognised reply id: {0}")]
    UnrecognisedReply(u64),

    #[error("Invalid reply from sub-message {id}, {err}")]
    ReplyParseFailure { id: u64, err: String },

    #[error("ExceedHardcap")]
    ExceedHardcap {},

    #[error("InvalidToken")]
    InvalidToken {},

    #[error("InvalidZeroAmount")]
    InvalidZeroAmount {},

    #[error("Unauthorized")]
    Unauthorized {},
}
