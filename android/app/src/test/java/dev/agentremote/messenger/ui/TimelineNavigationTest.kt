package dev.agentremote.messenger.ui

import dev.agentremote.messenger.model.TimelineContent
import dev.agentremote.messenger.model.TimelineItem
import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TimelineNavigationTest {
    @Test
    fun searchFindsMessagesInTimelineOrderAndExcludesActivityDetails() {
        val question = item(TimelineContent.UserMessage("请检查 Kotlin 编译错误"))
        val tool = item(TimelineContent.Command("kotlinc", null, "completed", 0, "Kotlin passed"))
        val answer = item(TimelineContent.AgentMessage("final", "KOTLIN 编译通过"))
        val timeline = listOf(question, tool, answer)

        assertEquals(listOf(question.id, answer.id), messageSearchMatches(timeline, " kotlin "))
        assertTrue(messageSearchMatches(timeline, "  ").isEmpty())
        assertTrue(messageSearchMatches(timeline, "未出现的词").isEmpty())
    }

    @Test
    fun searchRetainsMessageIdentityWhenStreamingTextChanges() {
        val answer = item(TimelineContent.AgentMessage("final", "构建"))
        val updated = answer.copy(revision = 2, content = TimelineContent.AgentMessage("final", "构建通过"))

        assertEquals(messageSearchMatches(listOf(answer), "构建"), messageSearchMatches(listOf(updated), "构建"))
    }

    @Test
    fun jumpLocatesMessageAfterGroupedActivity() {
        val question = item(TimelineContent.UserMessage("问题"))
        val command = item(TimelineContent.Command("cargo check", null, "completed", 0, null))
        val progress = item(TimelineContent.Progress("test", "测试", "completed", null))
        val answer = item(TimelineContent.AgentMessage("final", "检查通过"))
        val blocks = groupTimeline(listOf(question, command, progress, answer))

        assertEquals(2, timelineBlockIndex(blocks, answer.id))
        assertEquals(1, timelineBlockIndex(blocks, progress.id))
        assertEquals(-1, timelineBlockIndex(blocks, UUID.randomUUID()))
    }

    @Test
    fun completedTurnsAndResolvedRequestsDoNotCreateApprovalTodo() {
        val pending = item(TimelineContent.Approval(UUID.randomUUID(), "继续？", emptyList(), null))
        val resolved = item(TimelineContent.Approval(UUID.randomUUID(), "已确认", emptyList(), "allow"))
        val timeline = listOf(pending, resolved)

        assertEquals(listOf(pending), unresolvedApprovalItems(timeline, running = true))
        assertTrue(unresolvedApprovalItems(timeline, running = false).isEmpty())
    }

    private fun item(content: TimelineContent) = TimelineItem(
        id = UUID.randomUUID(),
        conversationId = UUID.fromString("11111111-1111-1111-1111-111111111111"),
        revision = 1,
        createdAtMs = 1,
        content = content,
    )
}
