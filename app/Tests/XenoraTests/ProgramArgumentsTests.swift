// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation
import Testing
@testable import Xenora

@MainActor
struct ProgramArgumentsTests {
    @Test func passesLiteralArgumentsInOrder() {
        let model = ProgramRunModel()
        let values = ["  two words  ", "", "--", "same", "same", "a\tb", #""quotes"\path"#]
        for (index, value) in values.enumerated() {
            _ = model.addArgument()
            model.arguments[index].value = value
        }
        let file = URL(fileURLWithPath: "/nonexistent/program.exe")
        let command = ProgramRunner.arguments(for: file, trace: true, programArguments: model.workerArguments)
        #expect(command == ["run", "--experimental", "--trace", "--pe", file.path, "--"] + values)
        #expect(Set(model.arguments.map(\.id)).count == values.count)
    }

    @Test func removingAnArgumentPreservesOtherRowsAndTheCapturedSnapshot() {
        let model = ProgramRunModel()
        let first = model.addArgument()
        let second = model.addArgument()
        let last = model.addArgument()
        model.arguments[0].value = "before"
        model.arguments[2].value = "last"
        let snapshot = model.workerArguments

        #expect(model.removeArgument(second) == last)
        model.arguments[0].value = "after"
        #expect(model.arguments.map(\.id) == [first, last])
        #expect(model.workerArguments == ["after", "last"])
        #expect(snapshot == ["before", "", "last"])
        #expect(model.removeArgument(last) == first)
        #expect(model.removeArgument(first) == nil)
    }

    @Test func distinguishesNoArgumentsFromOneEmptyArgument() {
        let model = ProgramRunModel()
        let file = URL(fileURLWithPath: "/nonexistent/program.exe")
        let base = ["run", "--experimental", "--pe", file.path]
        #expect(ProgramRunner.arguments(for: file, trace: false, programArguments: model.workerArguments) == base)
        _ = model.addArgument()
        #expect(ProgramRunner.arguments(for: file, trace: false, programArguments: model.workerArguments) == base + ["--", ""])
    }
}
