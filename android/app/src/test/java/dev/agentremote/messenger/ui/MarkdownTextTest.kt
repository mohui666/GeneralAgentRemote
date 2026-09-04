package dev.agentremote.messenger.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class MarkdownTextTest {
    @Test
    fun parsesCommonAgentMarkdownIntoDisplayBlocks() {
        val markdown = """
            # Result

            Use **bold**, *emphasis*, and `inline()`.

            - first
            - second

            ```kotlin
            val answer = 42
            ```
        """.trimIndent()

        val blocks = parseMarkdownBlocks(markdown)

        assertEquals(MarkdownBlock.Heading(1, "Result"), blocks[0])
        assertEquals(MarkdownBlock.Paragraph("Use **bold**, *emphasis*, and `inline()`."), blocks[1])
        assertEquals(MarkdownBlock.ListItems(false, listOf("first", "second")), blocks[2])
        assertEquals(MarkdownBlock.Code("kotlin", "val answer = 42"), blocks[3])
    }

    @Test
    fun parsesInlineStylesAndLinksWithoutEatingPlainText() {
        val inline = parseMarkdownInline("A **bold** *word*, `code`, and [docs](https://example.com).")

        assertEquals(
            listOf(
                MarkdownInline(MarkdownInlineKind.TEXT, "A "),
                MarkdownInline(MarkdownInlineKind.STRONG, "bold"),
                MarkdownInline(MarkdownInlineKind.TEXT, " "),
                MarkdownInline(MarkdownInlineKind.EMPHASIS, "word"),
                MarkdownInline(MarkdownInlineKind.TEXT, ", "),
                MarkdownInline(MarkdownInlineKind.CODE, "code"),
                MarkdownInline(MarkdownInlineKind.TEXT, ", and "),
                MarkdownInline(MarkdownInlineKind.LINK, "docs", "https://example.com"),
                MarkdownInline(MarkdownInlineKind.TEXT, "."),
            ),
            inline,
        )
    }

    @Test
    fun unterminatedFenceStillShowsItsCode() {
        val code = parseMarkdownBlocks("```\nanswer").single() as MarkdownBlock.Code

        assertNull(code.language)
        assertEquals("answer", code.text)
    }
}
