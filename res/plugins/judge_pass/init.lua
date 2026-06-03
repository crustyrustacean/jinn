-- Judge pass workflow — pushes a system message and turns off the workflow.
--
-- This is a one-shot workflow: it runs once, pushes a "judgement passed"
-- system message, and disables the workflow attachment so it won't fire again.

return {
    run = function(ctx)
        ctx.push_system("judgement passed")
        ctx.turn_off()
    end
}
