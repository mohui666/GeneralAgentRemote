package dev.agentremote.messenger.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp

private val MarkdownMuted = Color(0xFFA7A7AD)
private val MarkdownLink = Color(0xFFA477ED)
private val MarkdownFencePattern = Regex("^\\s*(```|~~~)(.*)$")
private val MarkdownHeadingPattern = Regex("^\\s*(#{1,6})\\s+(.+?)\\s*#*\\s*$")
private val MarkdownUnorderedPattern = Regex("^\\s*[-+*]\\s+(.+)$")
private val MarkdownOrderedPattern = Regex("^\\s*\\d+[.)]\\s+(.+)$")

internal sealed interface MarkdownBlock {
    data class Heading(val level: Int, val text: String) : MarkdownBlock
    data class Paragraph(val text: String) : MarkdownBlock
    data class Code(val language: String?, val text: String) : MarkdownBlock
    data class ListItems(val ordered: Boolean, val items: List<String>) : MarkdownBlock
}

internal enum class MarkdownInlineKind { TEXT, STRONG, EMPHASIS, CODE, LINK }

internal data class MarkdownInline(
    val kind: MarkdownInlineKind,
    val text: String,
    val destination: String? = null,
)

internal fun parseMarkdownBlocks(markdown: String): List<MarkdownBlock> {
    if (markdown.isEmpty()) return emptyList()
    val lines = markdown.replace("\r\n", "\n").replace('\r', '\n').split('\n')
    val blocks = mutableListOf<MarkdownBlock>()
    val paragraph = mutableListOf<String>()
    var index = 0

    fun flushParagraph() {
        if (paragraph.isNotEmpty()) {
            blocks += MarkdownBlock.Paragraph(paragraph.joinToString("\n"))
            paragraph.clear()
        }
    }

    while (index < lines.size) {
        val line = lines[index]
        val fence = MarkdownFencePattern.matchEntire(line)
        if (fence != null) {
            flushParagraph()
            val marker = fence.groupValues[1]
            val language = fence.groupValues[2].trim().ifEmpty { null }
            val code = mutableListOf<String>()
            index++
            while (index < lines.size && !lines[index].trimStart().startsWith(marker)) {
                code += lines[index]
                index++
            }
            if (index < lines.size) index++
            blocks += MarkdownBlock.Code(language, code.joinToString("\n"))
            continue
        }

        val heading = MarkdownHeadingPattern.matchEntire(line)
        if (heading != null) {
            flushParagraph()
            blocks += MarkdownBlock.Heading(heading.groupValues[1].length, heading.groupValues[2])
            index++
            continue
        }

        val unordered = MarkdownUnorderedPattern.matchEntire(line)
        val ordered = MarkdownOrderedPattern.matchEntire(line)
        if (unordered != null || ordered != null) {
            flushParagraph()
            val isOrdered = ordered != null
            val listItems = mutableListOf<String>()
            while (index < lines.size) {
                val item = if (isOrdered) {
                    MarkdownOrderedPattern.matchEntire(lines[index])
                } else {
                    MarkdownUnorderedPattern.matchEntire(lines[index])
                } ?: break
                listItems += item.groupValues[1]
                index++
            }
            blocks += MarkdownBlock.ListItems(isOrdered, listItems)
            continue
        }

        if (line.isBlank()) {
            flushParagraph()
        } else {
            paragraph += line
        }
        index++
    }
    flushParagraph()
    return blocks
}

internal fun parseMarkdownInline(text: String): List<MarkdownInline> {
    val spans = mutableListOf<MarkdownInline>()
    val plain = StringBuilder()
    var index = 0

    fun flushPlain() {
        if (plain.isNotEmpty()) {
            spans += MarkdownInline(MarkdownInlineKind.TEXT, plain.toString())
            plain.clear()
        }
    }

    while (index < text.length) {
        if (text[index] == '\\' && index + 1 < text.length) {
            plain.append(text[index + 1])
            index += 2
            continue
        }
        if (text[index] == '`') {
            val end = text.indexOf('`', index + 1)
            if (end > index + 1) {
                flushPlain()
                spans += MarkdownInline(MarkdownInlineKind.CODE, text.substring(index + 1, end))
                index = end + 1
                continue
            }
        }
        if (text[index] == '[') {
            val labelEnd = text.indexOf(']', index + 1)
            if (labelEnd > index + 1 && labelEnd + 1 < text.length && text[labelEnd + 1] == '(') {
                val destinationEnd = text.indexOf(')', labelEnd + 2)
                if (destinationEnd > labelEnd + 2) {
                    flushPlain()
                    spans += MarkdownInline(
                        MarkdownInlineKind.LINK,
                        text.substring(index + 1, labelEnd),
                        text.substring(labelEnd + 2, destinationEnd).trim(),
                    )
                    index = destinationEnd + 1
                    continue
                }
            }
        }
        val strongMarker = when {
            text.startsWith("**", index) -> "**"
            text.startsWith("__", index) -> "__"
            else -> null
        }
        if (strongMarker != null) {
            val end = text.indexOf(strongMarker, index + 2)
            if (end > index + 2) {
                flushPlain()
                spans += MarkdownInline(MarkdownInlineKind.STRONG, text.substring(index + 2, end))
                index = end + 2
                continue
            }
        }
        if (text[index] == '*' || text[index] == '_') {
            val marker = text[index]
            val end = text.indexOf(marker, index + 1)
            if (end > index + 1) {
                flushPlain()
                spans += MarkdownInline(MarkdownInlineKind.EMPHASIS, text.substring(index + 1, end))
                index = end + 1
                continue
            }
        }
        plain.append(text[index])
        index++
    }
    flushPlain()
    return spans
}

@Composable
internal fun MarkdownText(
    markdown: String,
    modifier: Modifier = Modifier,
    contentKey: String? = null,
) {
    val blocks = remember(markdown) { parseMarkdownBlocks(markdown) }
    val clipboard = LocalClipboardManager.current
    SelectionContainer {
        Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
            blocks.forEachIndexed { blockIndex, block ->
                when (block) {
                    is MarkdownBlock.Heading -> Text(
                        text = markdownAnnotatedString(block.text),
                        style = when (block.level) {
                            1 -> MaterialTheme.typography.headlineMedium
                            2 -> MaterialTheme.typography.titleLarge
                            else -> MaterialTheme.typography.titleMedium
                        },
                        fontWeight = FontWeight.SemiBold,
                    )
                    is MarkdownBlock.Paragraph -> Text(
                        text = markdownAnnotatedString(block.text),
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    is MarkdownBlock.Code -> Column(
                        Modifier
                            .fillMaxWidth()
                            .background(Color(0xFF101011), RoundedCornerShape(10.dp))
                            .padding(horizontal = 12.dp, vertical = 10.dp),
                        verticalArrangement = Arrangement.spacedBy(5.dp),
                    ) {
                        Row(
                            Modifier.fillMaxWidth().height(44.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                block.language ?: "代码",
                                modifier = Modifier.weight(1f),
                                color = MarkdownMuted,
                                style = MaterialTheme.typography.labelSmall,
                            )
                            IconButton(
                                onClick = { clipboard.setText(AnnotatedString(block.text)) },
                                modifier = Modifier
                                    .size(44.dp)
                                    .then(
                                        contentKey?.let {
                                            Modifier.testTag("gar.message.$it.code.$blockIndex.copy")
                                        } ?: Modifier,
                                    ),
                            ) {
                                Icon(
                                    Icons.Rounded.ContentCopy,
                                    contentDescription = "复制代码",
                                    modifier = Modifier.size(18.dp),
                                    tint = MarkdownMuted,
                                )
                            }
                        }
                        Text(
                            block.text,
                            modifier = Modifier.horizontalScroll(rememberScrollState()),
                            fontFamily = FontFamily.Monospace,
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                    is MarkdownBlock.ListItems -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        block.items.forEachIndexed { itemIndex, item ->
                            Row {
                                Text(
                                    if (block.ordered) "${itemIndex + 1}." else "•",
                                    modifier = Modifier.width(24.dp),
                                    color = MarkdownMuted,
                                    style = MaterialTheme.typography.bodyLarge,
                                )
                                Text(
                                    text = markdownAnnotatedString(item),
                                    modifier = Modifier.weight(1f),
                                    style = MaterialTheme.typography.bodyLarge,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun markdownAnnotatedString(text: String): AnnotatedString {
    val inline = remember(text) { parseMarkdownInline(text) }
    val linkStyle = remember {
        TextLinkStyles(
            style = SpanStyle(color = MarkdownLink, textDecoration = TextDecoration.Underline),
        )
    }
    return remember(inline, linkStyle) {
        buildAnnotatedString {
            inline.forEach { span ->
                when (span.kind) {
                    MarkdownInlineKind.TEXT -> append(span.text)
                    MarkdownInlineKind.STRONG -> withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(span.text) }
                    MarkdownInlineKind.EMPHASIS -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(span.text) }
                    MarkdownInlineKind.CODE -> withStyle(
                        SpanStyle(fontFamily = FontFamily.Monospace, background = Color(0xFF28282A)),
                    ) { append(span.text) }
                    MarkdownInlineKind.LINK -> {
                        val destination = span.destination.orEmpty()
                        if (destination.startsWith("https://") || destination.startsWith("http://") || destination.startsWith("mailto:")) {
                            withLink(LinkAnnotation.Url(destination, linkStyle)) { append(span.text) }
                        } else {
                            append(span.text)
                        }
                    }
                }
            }
        }
    }
}
