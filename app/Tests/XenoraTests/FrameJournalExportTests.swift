// SPDX-License-Identifier: GPL-3.0-or-later

import Foundation
import Testing
import UniformTypeIdentifiers
@testable import Xenora

struct FrameJournalExportTests {
    @Test func writesExactCsvInFrameOrder() throws {
        let document = try #require(FrameJournalDocument(records: [0x13, 0x20, 0x26, 0x29, 0x11]))
        let expected = "frame,column,mode,input\n"
            + "1,3,standard,none\n"
            + "2,0,interactive,neutral\n"
            + "3,2,interactive,right\n"
            + "4,1,interactive,left\n"
            + "5,1,standard,none\n"
        #expect(document.data == Data(expected.utf8))
        #expect(FrameJournalDocument.readableContentTypes == [.commaSeparatedText])
    }

    @Test func emptyJournalWritesOnlyTheHeader() throws {
        let document = try #require(FrameJournalDocument(records: []))
        #expect(document.data == Data("frame,column,mode,input\n".utf8))
    }

    @Test func rejectsUnknownJournalCodes() {
        let invalid: [UInt8] = [0x00, 0x14, 0x2c, 0x30, 0xa0, 0xff]
        for code in invalid {
            #expect(FrameJournalDocument(records: [0x10, code, 0x21]) == nil)
        }
    }

    @Test func documentKeepsAnImmutableSnapshot() throws {
        var records: [UInt8] = [0x24]
        let snapshot = try #require(FrameJournalDocument(records: records))
        records[0] = 0x28
        let updated = try #require(FrameJournalDocument(records: records))
        #expect(snapshot.data == Data("frame,column,mode,input\n1,0,interactive,right\n".utf8))
        #expect(updated.data == Data("frame,column,mode,input\n1,0,interactive,left\n".utf8))
    }
}
