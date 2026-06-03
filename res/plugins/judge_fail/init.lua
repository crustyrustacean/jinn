-- Judge fail workflow — pushes a user message telling the LLM to try again.
--
-- This is a one-shot workflow: it runs once per trigger, pushes a canned
-- "judgement failed" message, and returns. The workflow stays active so
-- it fires again on the next TurnEnd.

return {
    run = function(ctx)
        ctx.push_user("judgement failed, try again")
    end
}
