package core

type submitInput struct {
	OperationKey string    `json:"operation_key"`
	InputKind    inputKind `json:"input_kind"`
	Literal      string    `json:"literal"`
}
