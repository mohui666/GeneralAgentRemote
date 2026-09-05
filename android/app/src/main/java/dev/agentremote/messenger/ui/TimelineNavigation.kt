package dev.agentremote.messenger.ui

import dev.agentremote.messenger.model.TimelineContent
import dev.agentremote.messenger.model.TimelineItem
import java.util.UUID

internal fun messageSearchMatches(timeline: List<TimelineItem>, query: String): List<UUID> {
    val text = query.trim()
    if (text.isEmpty()) return emptyList()
    return timeline.mapNotNull { item ->
        val message = when (val content = item.content) {
            is TimelineContent.UserMessage -> content.text
            is TimelineContent.AgentMessage -> content.text
            else -> return@mapNotNull null
        }
        item.id.takeIf { message.contains(text, ignoreCase = true) }
    }
}

internal fun timelineBlockIndex(blocks: List<TimelineBlock>, itemId: UUID): Int =
    blocks.indexOfFirst { block ->
        when (block) {
            is TimelineBlock.Single -> block.item.id == itemId
            is TimelineBlock.Activity -> block.items.any { it.id == itemId }
        }
    }

internal fun unresolvedApprovalItems(timeline: List<TimelineItem>, running: Boolean): List<TimelineItem> =
    if (!running) emptyList() else timeline.filter {
        val approval = it.content as? TimelineContent.Approval
        approval != null && approval.resolvedOption == null
    }
