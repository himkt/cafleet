package core

import (
	"regexp"
	"strings"
)

var ansiEscapeRE = regexp.MustCompile(`\x1b\[[0-?]*[ -/]*[@-~]`)

func isVisuallyBlank(line string) bool {
	return strings.TrimSpace(ansiEscapeRE.ReplaceAllLiteralString(line, "")) == ""
}

func stripAnsi(text string) string {
	cleaned := ansiEscapeRE.ReplaceAllLiteralString(text, "")
	rows := strings.Split(cleaned, "\n")
	for i, row := range rows {
		row = strings.TrimRight(row, "\r")
		rows[i] = row[strings.LastIndexByte(row, '\r')+1:]
	}
	return strings.Join(rows, "\n")
}

func formatJSON(data any) string {
	encoded, err := encodeJSON(data)
	if err != nil {
		panic("a JSON payload always serializes: " + err.Error())
	}
	return string(encoded)
}

func truncateText(value string, limit int) string {
	count := 0
	for index := range value {
		if count == limit {
			return value[:index] + "…"
		}
		count++
	}
	return value
}
